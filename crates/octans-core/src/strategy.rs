//! `Strategy` — a node with several interchangeable implementations ("variants") that share one
//! boundary signature. Exactly one runs per tick; which one is chosen at runtime via a
//! [`StrategyHandle`]. This is R1's *switch* over implementations and the **autotuner's search
//! space**: each variant is an equivalent way to compute the same thing (CPU vs GPU vs combine),
//! and a tuner can pick the fastest (the profiler measures each; verify-by-default is a planned
//! follow-up once types carry a registered comparator).
//!
//! Each variant keeps its own state, so switching is cheap and a tuner can trial each in turn.

use crate::body::Body;
use crate::context::Context;
use crate::group::GroupTemplate;
use crate::node::{Inputs, Node, Outputs, PortSpec};
use crate::registry::Registry;
use std::any::Any;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// Builder for a [`Strategy`]; add variants by node or by group, then `build`.
#[derive(Default)]
pub struct StrategyBuilder {
    names: Vec<&'static str>,
    bodies: Vec<Body>,
}

impl StrategyBuilder {
    /// Add a variant that is a single node.
    pub fn node(mut self, name: &'static str, n: impl Node + 'static) -> Self {
        self.names.push(name);
        self.bodies.push(Body::from_node(Box::new(n)));
        self
    }

    /// Add a variant that is a whole group/subgraph.
    pub fn group(mut self, name: &'static str, template: &GroupTemplate) -> Self {
        self.names.push(name);
        self.bodies.push(Body::from_group(template));
        self
    }

    pub fn build(self) -> Strategy {
        assert!(
            !self.bodies.is_empty(),
            "a Strategy needs at least one variant"
        );
        for (i, b) in self.bodies.iter().enumerate().skip(1) {
            assert!(
                self.bodies[0].same_signature(b),
                "Strategy variant `{}` has a different boundary signature than `{}`",
                self.names[i],
                self.names[0]
            );
        }
        Strategy {
            names: self.names,
            variants: self.bodies,
            selected: Arc::new(AtomicUsize::new(0)),
        }
    }
}

pub struct Strategy {
    names: Vec<&'static str>,
    variants: Vec<Body>,
    selected: Arc<AtomicUsize>,
}

impl Strategy {
    pub fn builder() -> StrategyBuilder {
        StrategyBuilder::default()
    }

    /// A control handle to select the active variant (clone before adding to the graph).
    pub fn handle(&self) -> StrategyHandle {
        StrategyHandle {
            selected: Arc::clone(&self.selected),
            names: self.names.clone(),
        }
    }
}

/// Controls / inspects a [`Strategy`]'s active variant. Held by the author or the autotuner.
#[derive(Clone)]
pub struct StrategyHandle {
    selected: Arc<AtomicUsize>,
    names: Vec<&'static str>,
}

impl StrategyHandle {
    pub fn select(&self, variant: usize) {
        assert!(variant < self.names.len(), "variant index out of range");
        self.selected.store(variant, Ordering::Relaxed);
    }

    pub fn select_by_name(&self, name: &str) {
        let i = self
            .names
            .iter()
            .position(|&n| n == name)
            .expect("no such variant");
        self.select(i);
    }

    pub fn selected(&self) -> usize {
        self.selected.load(Ordering::Relaxed)
    }

    pub fn variant_count(&self) -> usize {
        self.names.len()
    }

    pub fn variant_name(&self, i: usize) -> &'static str {
        self.names[i]
    }
}

impl Node for Strategy {
    fn node_type(&self) -> &'static str {
        "octans.core.strategy"
    }

    fn inputs(&self) -> Vec<PortSpec> {
        self.variants[0]
            .inputs
            .iter()
            .map(|b| PortSpec::new(b.name, b.ty.clone()))
            .collect()
    }

    fn outputs(&self) -> Vec<PortSpec> {
        self.variants[0]
            .outputs
            .iter()
            .map(|b| PortSpec::new(b.name, b.ty.clone()))
            .collect()
    }

    fn prepare(&self, registry: &Registry) {
        for v in &self.variants {
            v.prepare(registry);
        }
    }

    /// One state set per variant (so switching variants doesn't lose state, and a tuner can
    /// trial each).
    fn new_local(&self) -> Box<dyn Any + Send> {
        Box::new(
            self.variants
                .iter()
                .map(|b| b.new_state())
                .collect::<Vec<Vec<Box<dyn Any + Send>>>>(),
        )
    }

    fn process(&self, ctx: &Context, local: &mut dyn Any, inputs: &Inputs, outputs: &mut Outputs) {
        let sel = self
            .selected
            .load(Ordering::Relaxed)
            .min(self.variants.len() - 1);
        let states = local
            .downcast_mut::<Vec<Vec<Box<dyn Any + Send>>>>()
            .expect("Strategy local state is per-variant state sets");
        let body = &self.variants[sel];

        let mut injected: HashMap<(usize, &'static str), crate::value::Value> = HashMap::new();
        for b in &body.inputs {
            injected.insert((b.node, b.port), inputs.value(b.name).clone());
        }
        let store = body.run(ctx, &mut states[sel], &injected);
        for b in &body.outputs {
            if let Some(v) = store.get(&(b.node, b.port)) {
                outputs.set_value(b.name, v.clone());
            }
        }
    }
}
