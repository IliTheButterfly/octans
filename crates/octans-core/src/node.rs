//! The node-authoring surface.
//!
//! A node's `process` receives three kinds of state, by exclusivity:
//! - `&self` — immutable logic/config, shared across all lanes (never cloned per lane).
//! - `&mut Local` (type-erased here as `&mut dyn Any`) — the node instance's **private** state,
//!   owned by the runtime and handed in exclusively (replicated per lane on fan-out, pinned if
//!   `!Send`). Accumulators, counters, controller memory.
//! - `&Context` — shared read-mostly globals (tick, resources).
//!
//! For v0 the erasure glue is written by hand; the `#[node]` macro generates it from a typed
//! `process` signature (`#[local] s: &mut State`, `#[ctx] ctx: &Context`, and input params).

use crate::context::Context;
use crate::value::{TypeSpec, Value};
use std::any::Any;
use std::collections::HashMap;

pub struct PortSpec {
    pub name: &'static str,
    pub ty: TypeSpec,
    /// For input ports: the value used when nothing is connected — i.e. a *parameter*. A future
    /// QoS/temporal contract (keep-last(N), required/optional) will live alongside this.
    pub default: Option<Value>,
}

impl PortSpec {
    pub fn new(name: &'static str, ty: TypeSpec) -> Self {
        Self {
            name,
            ty,
            default: None,
        }
    }

    /// An input port that falls back to `default` when unconnected (i.e. a parameter).
    pub fn with_default(name: &'static str, ty: TypeSpec, default: Value) -> Self {
        Self {
            name,
            ty,
            default: Some(default),
        }
    }
}

/// The type-erased values handed to a node for one tick, keyed by input-port name.
pub struct Inputs {
    pub(crate) map: HashMap<&'static str, Value>,
}

impl Inputs {
    pub fn value(&self, port: &str) -> &Value {
        self.map
            .get(port)
            .unwrap_or_else(|| panic!("missing input on port `{port}`"))
    }

    /// Typed accessor — the manual version of what `#[node]` generates.
    pub fn get<T: Any>(&self, port: &str) -> &T {
        self.value(port)
            .downcast_ref::<T>()
            .unwrap_or_else(|| panic!("input on port `{port}` had an unexpected type"))
    }
}

/// The values a node writes during a tick.
#[derive(Default)]
pub struct Outputs {
    pub(crate) map: HashMap<&'static str, Value>,
}

impl Outputs {
    pub fn set<T: Any + Send + Sync>(&mut self, port: &'static str, v: T) {
        self.map.insert(port, Value::new(v));
    }

    /// Set an output from an already-erased [`Value`] (used by portal/passthrough nodes).
    pub fn set_value(&mut self, port: &'static str, v: Value) {
        self.map.insert(port, v);
    }
}

/// A unit of computation. `&self` is immutable shared logic; mutable per-instance state is
/// handed in as `local`, and shared globals via `ctx`.
pub trait Node: Send + Sync {
    /// Stable node-type id, e.g. `"octans.std.threshold"`.
    fn node_type(&self) -> &'static str;
    fn inputs(&self) -> Vec<PortSpec>;
    fn outputs(&self) -> Vec<PortSpec>;

    /// Construct this instance's initial private state. Default: no state (`()`). The runtime
    /// owns the returned cell and replicates it per lane when the node is fanned out.
    fn new_local(&self) -> Box<dyn Any + Send> {
        Box::new(())
    }

    fn process(&self, ctx: &Context, local: &mut dyn Any, inputs: &Inputs, outputs: &mut Outputs);
}
