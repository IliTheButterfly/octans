//! The editable graph: node instances + typed connections.
//!
//! `connect` performs **connect-time type checking** and returns a diagnostic that points at
//! the offending nodes/ports — the perennial gap in every previous attempt
//! (`IncompatibleTypes` was defined but never enforced). Here it is enforced from day one.

use crate::context::Context;
use crate::node::{Inputs, Node, Outputs, PortSpec};
use crate::portal::Portal;
use crate::registry::Registry;
use crate::value::{TypeSpec, Value};
use std::any::Any;

/// Node-type id of the inert placeholder left behind by [`Graph::remove_node`].
pub const TOMBSTONE_TYPE: &str = "octans.core.tombstone";

/// A removed node's placeholder: no ports, does nothing. Replacing a removed node with this keeps
/// every other `NodeId` valid (they are positional indices), so deletion never renumbers.
struct Tombstone;
impl Node for Tombstone {
    fn node_type(&self) -> &'static str {
        TOMBSTONE_TYPE
    }
    fn inputs(&self) -> Vec<PortSpec> {
        Vec::new()
    }
    fn outputs(&self) -> Vec<PortSpec> {
        Vec::new()
    }
    fn process(&self, _: &Context, _: &mut dyn Any, _: &Inputs, _: &mut Outputs) {}
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NodeId(pub usize);

pub(crate) struct Edge {
    pub from_node: usize,
    pub from_port: &'static str,
    pub to_node: usize,
    pub to_port: &'static str,
}

/// A read-only view of one connection, for introspection by tooling (e.g. the GUI) that lives
/// outside this crate. Exposes [`NodeId`]s rather than the engine-internal positional indices.
#[derive(Clone, Copy, Debug)]
pub struct EdgeView {
    pub from: NodeId,
    pub from_port: &'static str,
    pub to: NodeId,
    pub to_port: &'static str,
}

/// A connection failure, carrying the ids/ports needed to light up the right node in a UI.
#[derive(Debug)]
pub enum ConnectError {
    NoSuchOutput {
        node: NodeId,
        port: String,
    },
    NoSuchInput {
        node: NodeId,
        port: String,
    },
    UnregisteredType {
        id: &'static str,
    },
    TypeMismatch {
        from_node: NodeId,
        from: TypeSpec,
        to_node: NodeId,
        to: TypeSpec,
    },
}

pub struct Graph {
    pub(crate) registry: Registry,
    pub(crate) nodes: Vec<Box<dyn Node>>,
    pub(crate) edges: Vec<Edge>,
    pub(crate) portals: Vec<Portal>,
}

impl Graph {
    pub fn new(registry: Registry) -> Self {
        Self {
            registry,
            nodes: Vec::new(),
            edges: Vec::new(),
            portals: Vec::new(),
        }
    }

    pub fn add(&mut self, node: impl Node + 'static) -> NodeId {
        self.nodes.push(Box::new(node));
        NodeId(self.nodes.len() - 1)
    }

    /// Add an already-boxed node (e.g. one built by a deserialization factory).
    pub fn add_boxed(&mut self, node: Box<dyn Node>) -> NodeId {
        self.nodes.push(node);
        NodeId(self.nodes.len() - 1)
    }

    /// Create a temporal feedback slot (z⁻¹). Use the returned [`Portal`] to make matching
    /// reader/writer nodes (`portal.reader(..)` / `portal.writer(..)`); the interpreter swaps
    /// it at each tick boundary so reads see the previous tick's write.
    pub fn add_portal(&mut self, ty: TypeSpec, initial: Value) -> Portal {
        let p = Portal::new(ty, initial);
        self.portals.push(p.clone());
        p
    }

    pub fn connect(
        &mut self,
        from: NodeId,
        from_port: &str,
        to: NodeId,
        to_port: &str,
    ) -> Result<(), ConnectError> {
        let edge = make_edge(&self.registry, &self.nodes, from, from_port, to, to_port)?;
        self.edges.push(edge);
        Ok(())
    }

    /// Validate a prospective connection without mutating the graph (same rules as `connect`).
    /// Lets a UI check a wire before committing — e.g. to refuse it, or to replace an existing
    /// edge only when the new one is actually valid.
    pub fn can_connect(
        &self,
        from: NodeId,
        from_port: &str,
        to: NodeId,
        to_port: &str,
    ) -> Result<(), ConnectError> {
        make_edge(&self.registry, &self.nodes, from, from_port, to, to_port).map(|_| ())
    }

    /// Replace a node in place, keeping its `NodeId` and its edges. Intended for parameter edits
    /// (which don't change a node's ports). If the new node's ports differ, stale edges may
    /// reference missing ports — that surfaces as a compile error, not corruption.
    pub fn replace_node(&mut self, id: NodeId, node: Box<dyn Node>) {
        if let Some(slot) = self.nodes.get_mut(id.0) {
            *slot = node;
        }
    }

    /// Remove a node, replacing it with an inert [`Tombstone`] and dropping every edge that touched
    /// it. Tombstoning keeps all other `NodeId`s valid — deletion must never renumber, since ids
    /// are positional indices that edges and engine state key on. The freed slot is not reused.
    pub fn remove_node(&mut self, id: NodeId) {
        if id.0 >= self.nodes.len() {
            return;
        }
        self.edges
            .retain(|e| e.from_node != id.0 && e.to_node != id.0);
        self.nodes[id.0] = Box::new(Tombstone);
    }

    /// Remove every edge feeding the input `(to, to_port)`; returns how many were removed. Safe
    /// w.r.t. `NodeId` indices — only edges are dropped, nodes are untouched (unlike node removal,
    /// which would renumber). Used by the editor to disconnect / replace a wire.
    pub fn disconnect_input(&mut self, to: NodeId, to_port: &str) -> usize {
        let before = self.edges.len();
        self.edges
            .retain(|e| !(e.to_node == to.0 && e.to_port == to_port));
        before - self.edges.len()
    }

    // --- read-only introspection (for tooling outside the crate, e.g. the GUI) ---

    /// Number of node instances in the (flattened) graph.
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// Iterate every node instance with its id, in insertion (`NodeId`) order.
    pub fn nodes(&self) -> impl Iterator<Item = (NodeId, &dyn Node)> + '_ {
        self.nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (NodeId(i), n.as_ref()))
    }

    /// Borrow a single node by id, if it exists.
    pub fn node(&self, id: NodeId) -> Option<&dyn Node> {
        self.nodes.get(id.0).map(|n| n.as_ref())
    }

    /// Iterate every connection as a read-only [`EdgeView`].
    pub fn edges(&self) -> impl Iterator<Item = EdgeView> + '_ {
        self.edges.iter().map(|e| EdgeView {
            from: NodeId(e.from_node),
            from_port: e.from_port,
            to: NodeId(e.to_node),
            to_port: e.to_port,
        })
    }
}

/// Validate a connection against the registry + node port specs, returning the edge (or a
/// diagnostic). Shared by `Graph::connect` and group flattening so the rules live in one place.
pub(crate) fn make_edge(
    registry: &Registry,
    nodes: &[Box<dyn Node>],
    from: NodeId,
    from_port: &str,
    to: NodeId,
    to_port: &str,
) -> Result<Edge, ConnectError> {
    let out = nodes[from.0]
        .outputs()
        .into_iter()
        .find(|p| p.name == from_port)
        .ok_or(ConnectError::NoSuchOutput {
            node: from,
            port: from_port.to_string(),
        })?;
    let inp = nodes[to.0]
        .inputs()
        .into_iter()
        .find(|p| p.name == to_port)
        .ok_or(ConnectError::NoSuchInput {
            node: to,
            port: to_port.to_string(),
        })?;

    if !registry.is_registered(out.ty.id) {
        return Err(ConnectError::UnregisteredType { id: out.ty.id });
    }
    if !registry.is_registered(inp.ty.id) {
        return Err(ConnectError::UnregisteredType { id: inp.ty.id });
    }
    if !out.ty.compatible_with(&inp.ty) {
        return Err(ConnectError::TypeMismatch {
            from_node: from,
            from: out.ty,
            to_node: to,
            to: inp.ty,
        });
    }

    // Store the nodes' own `'static` port names (the caller's `&str` may be borrowed/deserialized).
    Ok(Edge {
        from_node: from.0,
        from_port: out.name,
        to_node: to.0,
        to_port: inp.name,
    })
}
