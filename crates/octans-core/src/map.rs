//! `Map` — data-parallel fan-out.
//!
//! Applies a **body** to each element of a `Vector` input, producing a `Vector` output. The
//! body is either a single unary node ([`Map::new`]) or a whole group/subgraph ([`Map::group`]
//! — fan a pipeline over N cameras). Either way the body is run via the shared [`run_order`]
//! pass, with the lane's element **injected** at the body's input boundary.
//!
//! Each lane gets its **own** copy of the body's local state, and lanes run in **parallel** via
//! rayon — which compiles only because the state model is race-free: the body nodes are shared
//! `&` (Send+Sync logic), the context is shared `&` (Sync), and each lane holds disjoint `&mut`
//! to its own Send state. No locks.
//!
//! For a group body, the internal edges are validated against the registry in [`prepare`]
//! (where it's in scope), per the lazy-build-at-compile design. v1: exactly one boundary input
//! and one boundary output; no portals inside a mapped group body.

use crate::context::Context;
use crate::graph::{make_edge, Edge, NodeId};
use crate::group::GroupTemplate;
use crate::interp::{run_order, topo_order};
use crate::node::{Inputs, Node, Outputs, PortSpec};
use crate::registry::Registry;
use crate::value::{Shape, TypeSpec, Value};
use rayon::prelude::*;
use std::any::Any;
use std::collections::HashMap;
use std::sync::OnceLock;

/// An internal body connection awaiting validation (group bodies only).
struct BodyEdge {
    from: usize,
    from_port: &'static str,
    to: usize,
    to_port: &'static str,
}

pub struct Map {
    body_nodes: Vec<Box<dyn Node>>,
    body_order: Vec<usize>,
    pending: Vec<BodyEdge>, // unvalidated internal edges (empty for a single-node body)
    edges: OnceLock<Vec<Edge>>, // validated at prepare (set immediately for a single-node body)
    in_node: usize,
    in_port: &'static str,
    out_node: usize,
    out_port: &'static str,
    elem_in: TypeSpec,
    elem_out: TypeSpec,
}

impl Map {
    /// Map a single unary node (one required input, one output) over each element.
    pub fn new(inner: impl Node + 'static) -> Self {
        let inner: Box<dyn Node> = Box::new(inner);
        let ins = inner.inputs();
        let required: Vec<&PortSpec> = ins.iter().filter(|p| p.default.is_none()).collect();
        assert_eq!(
            required.len(),
            1,
            "Map::new wraps a unary node (exactly one required input); `{}` has {}",
            inner.node_type(),
            required.len()
        );
        let in_port = required[0].name;
        let elem_in = required[0].ty.clone();
        let outs = inner.outputs();
        assert_eq!(
            outs.len(),
            1,
            "Map::new wraps a node with exactly one output; `{}` has {}",
            inner.node_type(),
            outs.len()
        );
        let out_port = outs[0].name;
        let elem_out = outs[0].ty.clone();

        let edges = OnceLock::new();
        let _ = edges.set(Vec::new()); // no internal edges to validate

        Map {
            body_nodes: vec![inner],
            body_order: vec![0],
            pending: Vec::new(),
            edges,
            in_node: 0,
            in_port,
            out_node: 0,
            out_port,
            elem_in,
            elem_out,
        }
    }

    /// Map a whole group/subgraph over each element (fan a pipeline over N lanes).
    pub fn group(template: &GroupTemplate) -> Self {
        let gb = template.build_fresh();
        assert!(
            gb.portals.is_empty(),
            "Map::group v1 does not support portals inside a mapped body"
        );
        assert_eq!(
            gb.boundary_in.len(),
            1,
            "Map::group v1 needs exactly one boundary input"
        );
        assert_eq!(
            gb.boundary_out.len(),
            1,
            "Map::group v1 needs exactly one boundary output"
        );

        let (_, (in_node, in_port)) = gb.boundary_in.into_iter().next().unwrap();
        let (_, (out_node, out_port)) = gb.boundary_out.into_iter().next().unwrap();

        let elem_in = gb.nodes[in_node]
            .inputs()
            .into_iter()
            .find(|p| p.name == in_port)
            .expect("boundary input names a real inner port")
            .ty;
        let elem_out = gb.nodes[out_node]
            .outputs()
            .into_iter()
            .find(|p| p.name == out_port)
            .expect("boundary output names a real inner port")
            .ty;

        let pending: Vec<BodyEdge> = gb
            .edges
            .into_iter()
            .map(|e| BodyEdge {
                from: e.from,
                from_port: e.from_port,
                to: e.to,
                to_port: e.to_port,
            })
            .collect();
        let deps: Vec<(usize, usize)> = pending.iter().map(|e| (e.from, e.to)).collect();
        let body_order =
            topo_order(gb.nodes.len(), &deps).expect("mapped group body must be acyclic");

        Map {
            body_nodes: gb.nodes,
            body_order,
            pending,
            edges: OnceLock::new(),
            in_node,
            in_port,
            out_node,
            out_port,
            elem_in,
            elem_out,
        }
    }
}

impl Node for Map {
    fn node_type(&self) -> &'static str {
        "octans.core.map"
    }

    fn inputs(&self) -> Vec<PortSpec> {
        vec![PortSpec::new(
            "items",
            TypeSpec {
                id: self.elem_in.id,
                shape: Shape::Vector(None),
            },
        )]
    }

    fn outputs(&self) -> Vec<PortSpec> {
        vec![PortSpec::new(
            "items",
            TypeSpec {
                id: self.elem_out.id,
                shape: Shape::Vector(None),
            },
        )]
    }

    /// Validate the body's internal edges (registry in scope), and prepare body nodes.
    fn prepare(&self, registry: &Registry) {
        for n in &self.body_nodes {
            n.prepare(registry);
        }
        if self.edges.get().is_none() {
            let mut validated = Vec::with_capacity(self.pending.len());
            for e in &self.pending {
                match make_edge(
                    registry,
                    &self.body_nodes,
                    NodeId(e.from),
                    e.from_port,
                    NodeId(e.to),
                    e.to_port,
                ) {
                    Ok(edge) => validated.push(edge),
                    Err(err) => panic!("Map group body has an invalid connection: {err:?}"),
                }
            }
            let _ = self.edges.set(validated);
        }
    }

    /// Map's local state is the per-lane body states: `Vec<lane>` where `lane = Vec<node state>`.
    fn new_local(&self) -> Box<dyn Any + Send> {
        Box::new(Vec::<Vec<Box<dyn Any + Send>>>::new())
    }

    fn process(&self, ctx: &Context, local: &mut dyn Any, inputs: &Inputs, outputs: &mut Outputs) {
        let edges = self
            .edges
            .get()
            .expect("Map body not prepared — run via Mira::compile");
        let lanes = local
            .downcast_mut::<Vec<Vec<Box<dyn Any + Send>>>>()
            .expect("Map local state is per-lane body states");
        let items: &Vec<Value> = inputs.get::<Vec<Value>>("items");

        // Match the lane count to the input length (new lanes get fresh body state).
        while lanes.len() < items.len() {
            lanes.push(self.body_nodes.iter().map(|n| n.new_local()).collect());
        }
        lanes.truncate(items.len());

        let results: Vec<Value> = lanes
            .par_iter_mut()
            .zip(items.par_iter())
            .map(|(lane_locals, item)| {
                let mut injected: HashMap<(usize, &'static str), Value> = HashMap::new();
                injected.insert((self.in_node, self.in_port), item.clone());

                let store = run_order(
                    &self.body_nodes,
                    edges,
                    &self.body_order,
                    lane_locals,
                    ctx,
                    &injected,
                );
                store
                    .get(&(self.out_node, self.out_port))
                    .cloned()
                    .expect("group body produced its declared output")
            })
            .collect();

        outputs.set_value("items", Value::vector(results));
    }
}
