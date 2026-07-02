//! `Gather` / `Scatter` — pack N scalar streams into a `Vector`, and unpack a `Vector` back into
//! N scalar streams. These let independent sources (e.g. 5 separate camera nodes) feed a `Map`,
//! and let a `Map`'s vector result fan back out to per-lane consumers.
//!
//! Both are generic over the element type and have a *dynamic* port count, so their per-index
//! port names (`in0…`, `out0…`) are leaked to `&'static str` at construction (once per node, not
//! per tick) — fine for a graph built once. A future string-interning pass can replace this.

use crate::context::Context;
use crate::node::{Inputs, Node, Outputs, PortSpec};
use crate::value::{RegisteredType, Shape, TypeSpec, Value};
use std::any::Any;

fn leak(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

/// Pack N scalar inputs (`in0..in{N-1}`) into one `Vector` output (`items`).
pub struct Gather {
    elem: TypeSpec,
    in_names: Vec<&'static str>,
}

impl Gather {
    pub fn new<T: RegisteredType>(n: usize) -> Self {
        Self::new_dyn(T::type_spec(), n)
    }

    /// Construct with a runtime-chosen element type (no compile-time `T` — `Gather` is already
    /// type-erased internally). This is what lets an editor/palette build one from a type picked
    /// in a dropdown.
    pub fn new_dyn(elem: TypeSpec, n: usize) -> Self {
        Self {
            elem,
            in_names: (0..n).map(|i| leak(format!("in{i}"))).collect(),
        }
    }
}

impl Node for Gather {
    fn node_type(&self) -> &'static str {
        "octans.core.gather"
    }
    fn inputs(&self) -> Vec<PortSpec> {
        self.in_names
            .iter()
            .map(|&name| PortSpec::new(name, self.elem.clone()))
            .collect()
    }
    fn outputs(&self) -> Vec<PortSpec> {
        vec![PortSpec::new(
            "items",
            TypeSpec {
                id: self.elem.id,
                shape: Shape::Vector(Some(self.in_names.len())),
            },
        )]
    }
    fn process(
        &self,
        _ctx: &Context,
        _local: &mut dyn Any,
        inputs: &Inputs,
        outputs: &mut Outputs,
    ) {
        let items: Vec<Value> = self
            .in_names
            .iter()
            .map(|&n| inputs.value(n).clone())
            .collect();
        outputs.set_value("items", Value::vector(items));
    }
}

/// Unpack one `Vector` input (`items`) into N scalar outputs (`out0..out{N-1}`).
pub struct Scatter {
    elem: TypeSpec,
    out_names: Vec<&'static str>,
}

impl Scatter {
    pub fn new<T: RegisteredType>(n: usize) -> Self {
        Self::new_dyn(T::type_spec(), n)
    }

    /// Construct with a runtime-chosen element type (see [`Gather::new_dyn`]).
    pub fn new_dyn(elem: TypeSpec, n: usize) -> Self {
        Self {
            elem,
            out_names: (0..n).map(|i| leak(format!("out{i}"))).collect(),
        }
    }
}

impl Node for Scatter {
    fn node_type(&self) -> &'static str {
        "octans.core.scatter"
    }
    fn inputs(&self) -> Vec<PortSpec> {
        vec![PortSpec::new(
            "items",
            TypeSpec {
                id: self.elem.id,
                shape: Shape::Vector(Some(self.out_names.len())),
            },
        )]
    }
    fn outputs(&self) -> Vec<PortSpec> {
        self.out_names
            .iter()
            .map(|&name| PortSpec::new(name, self.elem.clone()))
            .collect()
    }
    fn process(
        &self,
        _ctx: &Context,
        _local: &mut dyn Any,
        inputs: &Inputs,
        outputs: &mut Outputs,
    ) {
        let items = inputs.get::<Vec<Value>>("items");
        for (i, &name) in self.out_names.iter().enumerate() {
            if let Some(v) = items.get(i) {
                outputs.set_value(name, v.clone());
            }
        }
    }
}
