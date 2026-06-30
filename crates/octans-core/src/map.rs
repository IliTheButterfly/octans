//! `Map` — data-parallel fan-out. Zips K input vectors through a [`Body`] into M output vectors,
//! one lane per element, in parallel (rayon). Each lane has its own copy of the body's state.
//!
//! The body is either a single node ([`Map::new`]) or a whole group ([`Map::group`]). Map's
//! ports are the body's boundaries, vectorized. That the parallel run compiles is the proof the
//! state model is race-free (shared `&` body + ctx, disjoint `&mut` per-lane state).

use crate::body::Body;
use crate::context::Context;
use crate::group::GroupTemplate;
use crate::node::{Inputs, Node, Outputs, PortSpec};
use crate::registry::Registry;
use crate::value::{Shape, TypeSpec, Value};
use rayon::prelude::*;
use std::any::Any;
use std::collections::HashMap;

pub struct Map {
    body: Body,
}

impl Map {
    /// Map a single node over each element (its required inputs are zipped).
    pub fn new(inner: impl Node + 'static) -> Self {
        Map {
            body: Body::from_node(Box::new(inner)),
        }
    }

    /// Map a whole group/subgraph over each element (its boundary inputs are zipped).
    pub fn group(template: &GroupTemplate) -> Self {
        Map {
            body: Body::from_group(template),
        }
    }
}

fn vectorized(b: &crate::body::Boundary) -> PortSpec {
    PortSpec::new(
        b.name,
        TypeSpec {
            id: b.ty.id,
            shape: Shape::Vector(None),
        },
    )
}

impl Node for Map {
    fn node_type(&self) -> &'static str {
        "octans.core.map"
    }

    fn inputs(&self) -> Vec<PortSpec> {
        self.body.inputs.iter().map(vectorized).collect()
    }

    fn outputs(&self) -> Vec<PortSpec> {
        self.body.outputs.iter().map(vectorized).collect()
    }

    fn prepare(&self, registry: &Registry) {
        self.body.prepare(registry);
    }

    /// Per-lane body states: `Vec<lane>` where `lane = Vec<node state>`.
    fn new_local(&self) -> Box<dyn Any + Send> {
        Box::new(Vec::<Vec<Box<dyn Any + Send>>>::new())
    }

    fn process(&self, ctx: &Context, local: &mut dyn Any, inputs: &Inputs, outputs: &mut Outputs) {
        let lanes = local
            .downcast_mut::<Vec<Vec<Box<dyn Any + Send>>>>()
            .expect("Map local state is per-lane body states");

        let vecs: Vec<&Vec<Value>> = self
            .body
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
            lanes.push(self.body.new_state());
        }
        lanes.truncate(n);

        let per_lane: Vec<Vec<Value>> = lanes
            .par_iter_mut()
            .enumerate()
            .map(|(i, lane)| {
                let mut injected: HashMap<(usize, &'static str), Value> = HashMap::new();
                for (k, b) in self.body.inputs.iter().enumerate() {
                    injected.insert((b.node, b.port), vecs[k][i].clone());
                }
                let store = self.body.run(ctx, lane, &injected);
                self.body
                    .outputs
                    .iter()
                    .map(|b| {
                        store
                            .get(&(b.node, b.port))
                            .cloned()
                            .expect("body produced its declared output")
                    })
                    .collect::<Vec<Value>>()
            })
            .collect();

        for (m, b) in self.body.outputs.iter().enumerate() {
            let col: Vec<Value> = per_lane.iter().map(|row| row[m].clone()).collect();
            outputs.set_value(b.name, Value::vector(col));
        }
    }
}
