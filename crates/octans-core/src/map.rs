//! `Map` — data-parallel fan-out.
//!
//! Wraps a *unary* inner node (one required input, one output) and applies it to each element
//! of a `Vector` input, producing a `Vector` output. The interesting part is state: each lane
//! gets its **own** copy of the inner node's local state, so a stateful inner (an accumulator,
//! a per-camera controller) tracks each element independently.
//!
//! Lanes run in **parallel** via rayon. That this compiles at all is the proof the state model
//! is race-free: the inner node is shared `&` (immutable logic, `Send + Sync`), the context is
//! shared `&` (`Sync`), and each lane gets a disjoint `&mut` to its own `Send` state. Shared
//! immutable logic + disjoint mutable per-lane state = safe concurrency, no locks.
//!
//! `Map`'s own local state IS the vector of per-lane inner states — so it rides the exact same
//! runtime-owned-local mechanism as any other node.
//!
//! v1 maps a node with exactly one required input (optional/param inputs use their defaults).
//! Zipped maps (per-lane parameters from a second vector) and subgraph bodies come later.

use crate::context::Context;
use crate::node::{Inputs, Node, Outputs, PortSpec};
use crate::value::{Shape, TypeSpec, Value};
use rayon::prelude::*;
use std::any::Any;
use std::collections::HashMap;

pub struct Map {
    inner: Box<dyn Node>,
    in_port: &'static str,
    out_port: &'static str,
    elem_in: TypeSpec,
    elem_out: TypeSpec,
    optional_defaults: Vec<(&'static str, Value)>,
}

impl Map {
    pub fn new(inner: impl Node + 'static) -> Self {
        Self::from_box(Box::new(inner))
    }

    pub fn from_box(inner: Box<dyn Node>) -> Self {
        let ins = inner.inputs();
        let required: Vec<&PortSpec> = ins.iter().filter(|p| p.default.is_none()).collect();
        assert_eq!(
            required.len(),
            1,
            "Map v1 wraps a unary node (exactly one required input); `{}` has {}",
            inner.node_type(),
            required.len()
        );
        let in_port = required[0].name;
        let elem_in = required[0].ty.clone();
        let optional_defaults = ins
            .iter()
            .filter_map(|p| p.default.clone().map(|d| (p.name, d)))
            .collect();

        let outs = inner.outputs();
        assert_eq!(
            outs.len(),
            1,
            "Map v1 wraps a node with exactly one output; `{}` has {}",
            inner.node_type(),
            outs.len()
        );
        let out_port = outs[0].name;
        let elem_out = outs[0].ty.clone();

        Self {
            inner,
            in_port,
            out_port,
            elem_in,
            elem_out,
            optional_defaults,
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

    /// Map's local state is the per-lane inner states (grown to match the input length).
    fn new_local(&self) -> Box<dyn Any + Send> {
        Box::new(Vec::<Box<dyn Any + Send>>::new())
    }

    fn process(&self, ctx: &Context, local: &mut dyn Any, inputs: &Inputs, outputs: &mut Outputs) {
        let lanes = local
            .downcast_mut::<Vec<Box<dyn Any + Send>>>()
            .expect("Map local state is a Vec of per-lane inner states");
        let items: &Vec<Value> = inputs.get::<Vec<Value>>("items");

        // Match the lane count to the input length (new lanes get a fresh inner state).
        while lanes.len() < items.len() {
            lanes.push(self.inner.new_local());
        }
        lanes.truncate(items.len());

        // One lane per element, in parallel. Disjoint `&mut` per lane; `&self`/`ctx` shared.
        let results: Vec<Value> = lanes
            .par_iter_mut()
            .zip(items.par_iter())
            .map(|(lane, item)| {
                let mut inmap: HashMap<&'static str, Value> = HashMap::new();
                inmap.insert(self.in_port, item.clone());
                for (name, def) in &self.optional_defaults {
                    inmap.insert(name, def.clone());
                }
                let inner_inputs = Inputs { map: inmap };
                let mut inner_outputs = Outputs::default();

                let state: &mut dyn Any = &mut **lane;
                self.inner
                    .process(ctx, state, &inner_inputs, &mut inner_outputs);

                inner_outputs
                    .map
                    .remove(self.out_port)
                    .expect("inner node produced its declared output")
            })
            .collect();

        outputs.set_value("items", Value::vector(results));
    }
}
