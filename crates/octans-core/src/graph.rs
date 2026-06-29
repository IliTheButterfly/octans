//! The editable graph: node instances + typed connections.
//!
//! `connect` performs **connect-time type checking** and returns a diagnostic that points at
//! the offending nodes/ports — the perennial gap in every previous attempt
//! (`IncompatibleTypes` was defined but never enforced). Here it is enforced from day one.

use crate::node::{Node, PortSpec};
use crate::portal::Portal;
use crate::registry::Registry;
use crate::value::{TypeSpec, Value};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NodeId(pub usize);

pub(crate) struct Edge {
    pub from_node: usize,
    pub from_port: &'static str,
    pub to_node: usize,
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

    /// Create a temporal feedback slot (z⁻¹). Use the returned [`Portal`] to make matching
    /// reader/writer nodes (`portal.reader(..)` / `portal.writer(..)`); the interpreter swaps
    /// it at each tick boundary so reads see the previous tick's write.
    pub fn add_portal(&mut self, ty: TypeSpec, initial: Value) -> Portal {
        let p = Portal::new(ty, initial);
        self.portals.push(p.clone());
        p
    }

    fn output_spec(&self, n: NodeId, port: &str) -> Option<PortSpec> {
        self.nodes[n.0]
            .outputs()
            .into_iter()
            .find(|p| p.name == port)
    }

    fn input_spec(&self, n: NodeId, port: &str) -> Option<PortSpec> {
        self.nodes[n.0]
            .inputs()
            .into_iter()
            .find(|p| p.name == port)
    }

    pub fn connect(
        &mut self,
        from: NodeId,
        from_port: &'static str,
        to: NodeId,
        to_port: &'static str,
    ) -> Result<(), ConnectError> {
        let out = self
            .output_spec(from, from_port)
            .ok_or(ConnectError::NoSuchOutput {
                node: from,
                port: from_port.to_string(),
            })?;
        let inp = self
            .input_spec(to, to_port)
            .ok_or(ConnectError::NoSuchInput {
                node: to,
                port: to_port.to_string(),
            })?;

        if !self.registry.is_registered(out.ty.id) {
            return Err(ConnectError::UnregisteredType { id: out.ty.id });
        }
        if !self.registry.is_registered(inp.ty.id) {
            return Err(ConnectError::UnregisteredType { id: inp.ty.id });
        }
        if out.ty != inp.ty {
            return Err(ConnectError::TypeMismatch {
                from_node: from,
                from: out.ty,
                to_node: to,
                to: inp.ty,
            });
        }

        self.edges.push(Edge {
            from_node: from.0,
            from_port,
            to_node: to.0,
            to_port,
        });
        Ok(())
    }
}
