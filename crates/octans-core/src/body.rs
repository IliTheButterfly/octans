//! `Body` — a runnable sub-pipeline (a single node or a whole group), shared by the constructs
//! that *run a body*: `Map` (fans it over N lanes) and `Strategy` (picks one of several).
//!
//! A body has typed **boundaries** (its external inputs/outputs) resolved to concrete inner
//! endpoints, plus the body's nodes/edges/topo-order. Group-body internal edges are validated
//! against the registry in [`Body::prepare`] (the lazy-build-at-compile design). Running a body
//! injects values at its input boundaries and reads its output boundaries from the store.

use crate::context::Context;
use crate::graph::{make_edge, Edge, NodeId};
use crate::group::GroupTemplate;
use crate::interp::{run_order, topo_order, Store};
use crate::node::Node;
use crate::registry::Registry;
use crate::value::{TypeSpec, Value};
use std::any::Any;
use std::collections::HashMap;
use std::sync::OnceLock;

/// A vectorizable boundary of a body: `name` is the body's port; `(node, port)` is the inner
/// endpoint it splices to.
pub(crate) struct Boundary {
    pub name: &'static str,
    pub node: usize,
    pub port: &'static str,
    pub ty: TypeSpec,
}

struct PendingEdge {
    from: usize,
    from_port: &'static str,
    to: usize,
    to_port: &'static str,
}

pub(crate) struct Body {
    nodes: Vec<Box<dyn Node>>,
    order: Vec<usize>,
    pending: Vec<PendingEdge>,
    edges: OnceLock<Vec<Edge>>,
    pub inputs: Vec<Boundary>,
    pub outputs: Vec<Boundary>,
}

impl Body {
    /// A body that is one unary node: required inputs / outputs become the boundaries.
    pub fn from_node(inner: Box<dyn Node>) -> Self {
        let inputs: Vec<Boundary> = inner
            .inputs()
            .into_iter()
            .filter(|p| p.default.is_none())
            .map(|p| Boundary {
                name: p.name,
                node: 0,
                port: p.name,
                ty: p.ty,
            })
            .collect();
        let outputs: Vec<Boundary> = inner
            .outputs()
            .into_iter()
            .map(|p| Boundary {
                name: p.name,
                node: 0,
                port: p.name,
                ty: p.ty,
            })
            .collect();
        assert!(
            !inputs.is_empty(),
            "body node `{}` needs at least one required input",
            inner.node_type()
        );
        assert!(!outputs.is_empty(), "body node needs at least one output");
        let edges = OnceLock::new();
        let _ = edges.set(Vec::new());
        Body {
            nodes: vec![inner],
            order: vec![0],
            pending: Vec::new(),
            edges,
            inputs,
            outputs,
        }
    }

    /// A body that is a whole group: boundary ports become the boundaries.
    pub fn from_group(template: &GroupTemplate) -> Self {
        let gb = template.build_fresh();
        assert!(
            gb.portals.is_empty(),
            "a body group cannot contain portals (v1)"
        );
        let resolve =
            |map: HashMap<&'static str, (usize, &'static str)>, is_input: bool| -> Vec<Boundary> {
                map.into_iter()
                    .map(|(name, (node, port))| {
                        let ty = if is_input {
                            gb.nodes[node].inputs().into_iter().find(|p| p.name == port)
                        } else {
                            gb.nodes[node]
                                .outputs()
                                .into_iter()
                                .find(|p| p.name == port)
                        }
                        .expect("boundary names a real inner port")
                        .ty;
                        Boundary {
                            name,
                            node,
                            port,
                            ty,
                        }
                    })
                    .collect()
            };
        let inputs = resolve(gb.boundary_in, true);
        let outputs = resolve(gb.boundary_out, false);
        assert!(!inputs.is_empty(), "group body needs >= 1 boundary input");
        assert!(!outputs.is_empty(), "group body needs >= 1 boundary output");

        let pending: Vec<PendingEdge> = gb
            .edges
            .into_iter()
            .map(|e| PendingEdge {
                from: e.from,
                from_port: e.from_port,
                to: e.to,
                to_port: e.to_port,
            })
            .collect();
        let deps: Vec<(usize, usize)> = pending.iter().map(|e| (e.from, e.to)).collect();
        let order = topo_order(gb.nodes.len(), &deps).expect("body must be acyclic");

        Body {
            nodes: gb.nodes,
            order,
            pending,
            edges: OnceLock::new(),
            inputs,
            outputs,
        }
    }

    /// Validate internal edges against the registry and prepare body nodes (compile-time).
    pub fn prepare(&self, registry: &Registry) {
        for n in &self.nodes {
            n.prepare(registry);
        }
        if self.edges.get().is_none() {
            let mut validated = Vec::with_capacity(self.pending.len());
            for e in &self.pending {
                match make_edge(
                    registry,
                    &self.nodes,
                    NodeId(e.from),
                    e.from_port,
                    NodeId(e.to),
                    e.to_port,
                ) {
                    Ok(edge) => validated.push(edge),
                    Err(err) => panic!("body has an invalid connection: {err:?}"),
                }
            }
            let _ = self.edges.set(validated);
        }
    }

    /// A fresh per-node state set for one instance/lane of this body.
    pub fn new_state(&self) -> Vec<Box<dyn Any + Send>> {
        self.nodes.iter().map(|n| n.new_local()).collect()
    }

    /// Run the body once with `locals` (its per-node state) and `injected` boundary inputs.
    pub fn run(
        &self,
        ctx: &Context,
        locals: &mut [Box<dyn Any + Send>],
        injected: &HashMap<(usize, &'static str), Value>,
    ) -> Store {
        let edges = self
            .edges
            .get()
            .expect("body not prepared (run via Mira::compile)");
        let mut timings = Vec::new();
        run_order(
            &self.nodes,
            edges,
            &self.order,
            locals,
            ctx,
            injected,
            &mut timings,
        )
    }

    /// True if `self` and `other` have the same boundary signature (port names + types).
    pub fn same_signature(&self, other: &Body) -> bool {
        let sig = |bs: &[Boundary]| -> Vec<(&'static str, TypeSpec)> {
            let mut v: Vec<_> = bs.iter().map(|b| (b.name, b.ty.clone())).collect();
            v.sort_by_key(|(n, _)| *n);
            v
        };
        sig(&self.inputs) == sig(&other.inputs) && sig(&self.outputs) == sig(&other.outputs)
    }
}
