//! `Map` — data-parallel fan-out, zipping K input vectors through a body into M output vectors.
//!
//! The body is either a single node ([`Map::new`]) or a whole group/subgraph ([`Map::group`]).
//! Map exposes one `Vector` input port per body boundary input (named after it) and one `Vector`
//! output port per boundary output. Per lane `i` it injects element `i` of every input vector at
//! the matching boundary, runs the body via [`run_order`], and collects each boundary output —
//! i.e. it *zips* the inputs (e.g. per-camera `frames` ⨯ `calibrations`).
//!
//! Each lane gets its own copy of the body's local state; lanes run in **parallel** via rayon —
//! which compiles only because the state model is race-free (shared `&` Send+Sync body + ctx,
//! disjoint `&mut` per-lane state). A group body's internal edges are validated against the
//! registry in [`prepare`] (the lazy-build-at-compile design).
//!
//! v1: no portals inside a mapped group body; all input vectors must share a length.

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

struct BodyEdge {
    from: usize,
    from_port: &'static str,
    to: usize,
    to_port: &'static str,
}

/// A vectorized boundary of the body: `name` is Map's port; `(node, port)` is the inner endpoint.
struct Boundary {
    name: &'static str,
    node: usize,
    port: &'static str,
    ty: TypeSpec,
}

pub struct Map {
    body_nodes: Vec<Box<dyn Node>>,
    body_order: Vec<usize>,
    pending: Vec<BodyEdge>,
    edges: OnceLock<Vec<Edge>>,
    inputs: Vec<Boundary>,  // zipped input vectors
    outputs: Vec<Boundary>, // result vectors
}

impl Map {
    /// Map a single node over each element. Its **required** inputs become zipped input vectors
    /// (optional/param inputs use their defaults per lane); its outputs become result vectors.
    pub fn new(inner: impl Node + 'static) -> Self {
        let inner: Box<dyn Node> = Box::new(inner);
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
            "Map::new needs a node with at least one required input; `{}` has none",
            inner.node_type()
        );
        assert!(
            !outputs.is_empty(),
            "Map::new needs a node with at least one output"
        );

        let edges = OnceLock::new();
        let _ = edges.set(Vec::new());
        Map {
            body_nodes: vec![inner],
            body_order: vec![0],
            pending: Vec::new(),
            edges,
            inputs,
            outputs,
        }
    }

    /// Map a whole group/subgraph over each element. Its boundary inputs become zipped input
    /// vectors; its boundary outputs become result vectors.
    pub fn group(template: &GroupTemplate) -> Self {
        let gb = template.build_fresh();
        assert!(
            gb.portals.is_empty(),
            "Map::group v1 does not support portals inside a mapped body"
        );

        let boundary =
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
        let inputs = boundary(gb.boundary_in, true);
        let outputs = boundary(gb.boundary_out, false);
        assert!(
            !inputs.is_empty(),
            "Map::group needs at least one boundary input"
        );
        assert!(
            !outputs.is_empty(),
            "Map::group needs at least one boundary output"
        );

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
            inputs,
            outputs,
        }
    }
}

impl Node for Map {
    fn node_type(&self) -> &'static str {
        "octans.core.map"
    }

    fn inputs(&self) -> Vec<PortSpec> {
        self.inputs
            .iter()
            .map(|b| {
                PortSpec::new(
                    b.name,
                    TypeSpec {
                        id: b.ty.id,
                        shape: Shape::Vector(None),
                    },
                )
            })
            .collect()
    }

    fn outputs(&self) -> Vec<PortSpec> {
        self.outputs
            .iter()
            .map(|b| {
                PortSpec::new(
                    b.name,
                    TypeSpec {
                        id: b.ty.id,
                        shape: Shape::Vector(None),
                    },
                )
            })
            .collect()
    }

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

        // Gather + zip the input vectors (all must share a length).
        let vecs: Vec<&Vec<Value>> = self
            .inputs
            .iter()
            .map(|b| inputs.get::<Vec<Value>>(b.name))
            .collect();
        let n = vecs[0].len();
        assert!(
            vecs.iter().all(|v| v.len() == n),
            "Map zip: all input vectors must share a length"
        );

        while lanes.len() < n {
            lanes.push(self.body_nodes.iter().map(|nd| nd.new_local()).collect());
        }
        lanes.truncate(n);

        // Per lane: inject every input element, run the body, collect every output. (M values/lane.)
        let per_lane: Vec<Vec<Value>> = lanes
            .par_iter_mut()
            .enumerate()
            .map(|(i, lane)| {
                let mut injected: HashMap<(usize, &'static str), Value> = HashMap::new();
                for (k, b) in self.inputs.iter().enumerate() {
                    injected.insert((b.node, b.port), vecs[k][i].clone());
                }
                let mut timings = Vec::new(); // top-level profiler measures the Map node as a whole
                let store = run_order(
                    &self.body_nodes,
                    edges,
                    &self.body_order,
                    lane,
                    ctx,
                    &injected,
                    &mut timings,
                );
                self.outputs
                    .iter()
                    .map(|b| {
                        store
                            .get(&(b.node, b.port))
                            .cloned()
                            .expect("group body produced its declared output")
                    })
                    .collect::<Vec<Value>>()
            })
            .collect();

        // Transpose lanes×outputs into one result vector per output port.
        for (m, b) in self.outputs.iter().enumerate() {
            let col: Vec<Value> = per_lane.iter().map(|row| row[m].clone()).collect();
            outputs.set_value(b.name, Value::vector(col));
        }
    }
}
