//! Groups / subgraphs — an **edit-time** construct that **flattens** into the runtime plan.
//!
//! A [`GroupTemplate`] is a *builder*: a fn that populates a fresh subgraph (child nodes,
//! internal connections, internal portals) and declares **boundary ports** — the group's
//! external interface. Each boundary port has two faces: an **inner** face (wired to a child
//! inside the template) and an **outer** face (bound to the surrounding graph at instantiation).
//!
//! Instantiating a group ([`Graph::add_group`]) **inlines** its children into the parent and
//! resolves each boundary port to the concrete inner endpoint it stands for. So a boundary port
//! is a compile-time *edge splice*, never a runtime node — which is exactly why the historical
//! "passthrough port" problem (which stalled the C++ attempt at runtime) simply doesn't arise.
//!
//! Connections inside a builder are recorded untyped and validated when the group is inlined
//! (the parent's registry is in scope then). Each instantiation runs the builder afresh, so two
//! instances get independent child state and independent portals.
//!
//! v1: a boundary port maps 1:1 to one inner (child, port). Map-over-a-group (a group as a
//! fan-out body) and serialization come later.

use crate::graph::{make_edge, ConnectError, Edge, Graph, NodeId};
use crate::node::Node;
use crate::portal::Portal;
use crate::value::{TypeSpec, Value};
use std::collections::HashMap;

/// A connection recorded inside a builder (local indices), validated at inline time.
pub(crate) struct PendingEdge {
    pub(crate) from: usize,
    pub(crate) from_port: &'static str,
    pub(crate) to: usize,
    pub(crate) to_port: &'static str,
}

/// Accumulates a group body: child nodes, internal (unchecked) connections, internal portals,
/// and the boundary-port → inner-endpoint maps.
#[derive(Default)]
pub struct GroupBuilder {
    pub(crate) nodes: Vec<Box<dyn Node>>,
    pub(crate) edges: Vec<PendingEdge>,
    pub(crate) portals: Vec<Portal>,
    pub(crate) boundary_in: HashMap<&'static str, (usize, &'static str)>,
    pub(crate) boundary_out: HashMap<&'static str, (usize, &'static str)>,
}

impl GroupBuilder {
    pub fn add(&mut self, node: impl Node + 'static) -> NodeId {
        self.nodes.push(Box::new(node));
        NodeId(self.nodes.len() - 1)
    }

    /// Add an already-boxed node (e.g. one built by a deserialization factory).
    pub fn add_boxed(&mut self, node: Box<dyn Node>) -> NodeId {
        self.nodes.push(node);
        NodeId(self.nodes.len() - 1)
    }

    /// Record an internal connection (validated when the group is inlined).
    pub fn connect(
        &mut self,
        from: NodeId,
        from_port: &'static str,
        to: NodeId,
        to_port: &'static str,
    ) {
        self.edges.push(PendingEdge {
            from: from.0,
            from_port,
            to: to.0,
            to_port,
        });
    }

    /// An internal temporal feedback slot. Fresh per instantiation (independent feedback).
    pub fn add_portal(&mut self, ty: TypeSpec, initial: Value) -> Portal {
        let p = Portal::new(ty, initial);
        self.portals.push(p.clone());
        p
    }

    /// Nest another group: inline its body into this builder, returning the nested instance's
    /// boundary endpoints (in this builder's local index space).
    pub fn add_group(&mut self, template: &GroupTemplate) -> GroupInstance {
        let sub = template.build_fresh();
        let base = self.nodes.len();
        self.nodes.extend(sub.nodes);
        self.portals.extend(sub.portals);
        for e in sub.edges {
            self.edges.push(PendingEdge {
                from: e.from + base,
                from_port: e.from_port,
                to: e.to + base,
                to_port: e.to_port,
            });
        }
        GroupInstance::from_boundaries(base, sub.boundary_in, sub.boundary_out)
    }

    /// Declare a boundary **input**: the group input `name` feeds child `node`'s `port`.
    pub fn input(&mut self, name: &'static str, node: NodeId, port: &'static str) {
        self.boundary_in.insert(name, (node.0, port));
    }

    /// Declare a boundary **output**: the group output `name` is produced by child `node`'s `port`.
    pub fn output(&mut self, name: &'static str, node: NodeId, port: &'static str) {
        self.boundary_out.insert(name, (node.0, port));
    }
}

/// A reusable group definition. Build it with [`group`]; instantiate it many times.
pub struct GroupTemplate {
    pub name: &'static str,
    build: Box<dyn Fn(&mut GroupBuilder)>,
}

impl GroupTemplate {
    pub(crate) fn build_fresh(&self) -> GroupBuilder {
        let mut gb = GroupBuilder::default();
        (self.build)(&mut gb);
        gb
    }

    /// Lower a **data-defined** group body ([`BodySpec`]) into a live template, rebuilding its
    /// nodes through `factories` on every instantiation (each instance gets fresh state, exactly
    /// like a closure-built group). This is how editor-authored / deserialized groups exist —
    /// closures can't be saved, data can.
    ///
    /// All node types are validated up front (`BuildError::UnknownNodeType` if a factory is
    /// missing); edges/boundaries are validated at instantiation like any group. Port and
    /// boundary names are leaked to `'static` once per template (bounded by the spec's size).
    pub fn from_spec(
        name: &str,
        spec: crate::serial::BodySpec,
        factories: std::sync::Arc<crate::serial::NodeRegistry>,
    ) -> Result<GroupTemplate, crate::serial::BuildError> {
        fn leak(s: &str) -> &'static str {
            Box::leak(s.to_string().into_boxed_str())
        }

        // Validate every node type once, so instantiation can't fail on an unknown type.
        for n in &spec.nodes {
            if factories.build(&n.type_id, &n.config).is_none() {
                return Err(crate::serial::BuildError::UnknownNodeType(
                    n.type_id.clone(),
                ));
            }
        }

        // Pre-leak names so the replay closure is allocation-light and infallible.
        let nodes: Vec<(String, serde_json::Value)> = spec
            .nodes
            .into_iter()
            .map(|n| (n.type_id, n.config))
            .collect();
        let edges: Vec<(usize, &'static str, usize, &'static str)> = spec
            .edges
            .iter()
            .map(|e| (e.from, leak(&e.from_port), e.to, leak(&e.to_port)))
            .collect();
        let inputs: Vec<(&'static str, usize, &'static str)> = spec
            .inputs
            .iter()
            .map(|b| (leak(&b.name), b.node, leak(&b.port)))
            .collect();
        let outputs: Vec<(&'static str, usize, &'static str)> = spec
            .outputs
            .iter()
            .map(|b| (leak(&b.name), b.node, leak(&b.port)))
            .collect();

        Ok(GroupTemplate {
            name: leak(name),
            build: Box::new(move |gb: &mut GroupBuilder| {
                for (type_id, config) in &nodes {
                    let node = factories
                        .build(type_id, config)
                        .expect("validated at from_spec time");
                    gb.add_boxed(node);
                }
                for &(from, fp, to, tp) in &edges {
                    gb.connect(NodeId(from), fp, NodeId(to), tp);
                }
                for &(name, node, port) in &inputs {
                    gb.input(name, NodeId(node), port);
                }
                for &(name, node, port) in &outputs {
                    gb.output(name, NodeId(node), port);
                }
            }),
        })
    }
}

/// Define a reusable group template from a builder closure.
pub fn group(name: &'static str, build: impl Fn(&mut GroupBuilder) + 'static) -> GroupTemplate {
    GroupTemplate {
        name,
        build: Box::new(build),
    }
}

/// A placed group: its boundary ports, resolved to the concrete inner endpoints they splice to.
pub struct GroupInstance {
    inputs: HashMap<&'static str, (NodeId, &'static str)>,
    outputs: HashMap<&'static str, (NodeId, &'static str)>,
}

impl GroupInstance {
    fn from_boundaries(
        base: usize,
        bin: HashMap<&'static str, (usize, &'static str)>,
        bout: HashMap<&'static str, (usize, &'static str)>,
    ) -> Self {
        let inputs = bin
            .into_iter()
            .map(|(n, (i, p))| (n, (NodeId(i + base), p)))
            .collect();
        let outputs = bout
            .into_iter()
            .map(|(n, (i, p))| (n, (NodeId(i + base), p)))
            .collect();
        Self { inputs, outputs }
    }

    /// The concrete inner sink behind boundary input `name` — connect a producer to it.
    pub fn input(&self, name: &str) -> (NodeId, &'static str) {
        *self
            .inputs
            .get(name)
            .unwrap_or_else(|| panic!("group has no input port `{name}`"))
    }

    /// The concrete inner source behind boundary output `name` — connect it to a consumer.
    pub fn output(&self, name: &str) -> (NodeId, &'static str) {
        *self
            .outputs
            .get(name)
            .unwrap_or_else(|| panic!("group has no output port `{name}`"))
    }
}

impl Graph {
    /// Instantiate a group: inline its children into this graph (validating internal
    /// connections), and return the instance's boundary endpoints for wiring.
    pub fn add_group(&mut self, template: &GroupTemplate) -> Result<GroupInstance, ConnectError> {
        let sub = template.build_fresh();
        let base = self.nodes.len();
        self.nodes.extend(sub.nodes);
        self.portals.extend(sub.portals);

        // Validate + lower each internal connection against the parent registry.
        for e in sub.edges {
            let edge: Edge = make_edge(
                &self.registry,
                &self.nodes,
                NodeId(e.from + base),
                e.from_port,
                NodeId(e.to + base),
                e.to_port,
            )?;
            self.edges.push(edge);
        }

        Ok(GroupInstance::from_boundaries(
            base,
            sub.boundary_in,
            sub.boundary_out,
        ))
    }
}
