//! Control-flow base nodes: `Switch` (route one of N inputs) and `Loop` (bounded iteration of a
//! body). Two of R1's base components.

use crate::body::Body;
use crate::context::Context;
use crate::group::GroupTemplate;
use crate::node::{Inputs, Node, Outputs, PortSpec};
use crate::registry::Registry;
use crate::value::{RegisteredType, TypeSpec, Value};
use std::any::Any;
use std::collections::HashMap;

fn leak(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

/// Forward one of N inputs (`in0..in{N-1}`) to `out`, chosen by the `select: u32` input.
/// (All inputs are still evaluated upstream; `Switch` only routes the value. To run only the
/// selected *computation*, use a `Strategy`.)
pub struct Switch {
    elem: TypeSpec,
    in_names: Vec<&'static str>,
}

impl Switch {
    pub fn new<T: RegisteredType>(n: usize) -> Self {
        Self {
            elem: T::type_spec(),
            in_names: (0..n).map(|i| leak(format!("in{i}"))).collect(),
        }
    }
}

impl Node for Switch {
    fn node_type(&self) -> &'static str {
        "octans.core.switch"
    }
    fn inputs(&self) -> Vec<PortSpec> {
        let mut v = vec![PortSpec::new(
            "select",
            TypeSpec::scalar(<u32 as RegisteredType>::ID),
        )];
        v.extend(
            self.in_names
                .iter()
                .map(|&n| PortSpec::new(n, self.elem.clone())),
        );
        v
    }
    fn outputs(&self) -> Vec<PortSpec> {
        vec![PortSpec::new("out", self.elem.clone())]
    }
    fn process(
        &self,
        _ctx: &Context,
        _local: &mut dyn Any,
        inputs: &Inputs,
        outputs: &mut Outputs,
    ) {
        let sel =
            (*inputs.get::<u32>("select") as usize).min(self.in_names.len().saturating_sub(1));
        let v = inputs.value(self.in_names[sel]).clone();
        outputs.set_value("out", v);
    }
}

/// Apply a body `count` times, threading its output back as its next input (so the body's
/// single input and single output must share a type). Bounded iteration; the body's local state
/// persists across iterations and ticks.
pub struct Loop {
    body: Body,
    count: usize,
}

impl Loop {
    pub fn new(count: usize, inner: impl Node + 'static) -> Self {
        Self::from_body(Body::from_node(Box::new(inner)), count)
    }

    pub fn group(count: usize, template: &GroupTemplate) -> Self {
        Self::from_body(Body::from_group(template), count)
    }

    fn from_body(body: Body, count: usize) -> Self {
        assert_eq!(body.inputs.len(), 1, "Loop body needs exactly one input");
        assert_eq!(body.outputs.len(), 1, "Loop body needs exactly one output");
        assert!(
            body.inputs[0].ty == body.outputs[0].ty,
            "Loop body input and output types must match (the output feeds back as the input)"
        );
        Self { body, count }
    }
}

impl Node for Loop {
    fn node_type(&self) -> &'static str {
        "octans.core.loop"
    }
    fn inputs(&self) -> Vec<PortSpec> {
        vec![PortSpec::new(
            self.body.inputs[0].name,
            self.body.inputs[0].ty.clone(),
        )]
    }
    fn outputs(&self) -> Vec<PortSpec> {
        vec![PortSpec::new(
            self.body.outputs[0].name,
            self.body.outputs[0].ty.clone(),
        )]
    }
    fn prepare(&self, registry: &Registry) {
        self.body.prepare(registry);
    }
    fn new_local(&self) -> Box<dyn Any + Send> {
        Box::new(self.body.new_state())
    }
    fn process(&self, ctx: &Context, local: &mut dyn Any, inputs: &Inputs, outputs: &mut Outputs) {
        let state = local
            .downcast_mut::<Vec<Box<dyn Any + Send>>>()
            .expect("Loop local state is the body's state set");
        let ib = &self.body.inputs[0];
        let ob = &self.body.outputs[0];

        let mut val = inputs.value(ib.name).clone();
        for _ in 0..self.count {
            let mut injected: HashMap<(usize, &'static str), Value> = HashMap::new();
            injected.insert((ib.node, ib.port), val);
            let store = self.body.run(ctx, state, &injected);
            val = store
                .get(&(ob.node, ob.port))
                .cloned()
                .expect("loop body produced its output");
        }
        outputs.set_value(ob.name, val);
    }
}
