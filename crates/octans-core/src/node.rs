//! The node-authoring surface.
//!
//! For v0 the type-erasure "glue" (pulling typed values out of the inputs, pushing typed
//! values into the outputs) is written by hand. The eventual `#[node]` macro (open fork F1)
//! will generate exactly this glue — and we discover the macro's shape by writing real nodes
//! concretely first, rather than designing it in a vacuum.

use crate::value::{TypeSpec, Value};
use std::any::Any;
use std::collections::HashMap;

pub struct PortSpec {
    pub name: &'static str,
    pub ty: TypeSpec,
    /// For input ports: the value used when nothing is connected. This is what turns an input
    /// into a *parameter* — an optimizer can drive it via a connection, or it falls back to
    /// this default. (A future QoS/temporal contract — keep-last(N), required/optional — will
    /// live alongside this.)
    pub default: Option<Value>,
}

impl PortSpec {
    pub fn new(name: &'static str, ty: TypeSpec) -> Self {
        Self { name, ty, default: None }
    }

    /// An input port that falls back to `default` when unconnected (i.e. a parameter).
    pub fn with_default(name: &'static str, ty: TypeSpec, default: Value) -> Self {
        Self { name, ty, default: Some(default) }
    }
}

/// The type-erased values handed to a node for one tick, keyed by input-port name.
///
/// Values are owned (cheap `Arc` clones), so a node never holds a borrow into the engine's
/// value store while it runs.
pub struct Inputs {
    pub(crate) map: HashMap<&'static str, Value>,
}

impl Inputs {
    pub fn value(&self, port: &str) -> &Value {
        self.map
            .get(port)
            .unwrap_or_else(|| panic!("missing input on port `{port}`"))
    }

    /// Typed accessor — the manual version of what `#[node]` will generate.
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

/// A unit of computation. Nodes are `Send + Sync` so the future scheduler can place them on
/// any worker/thread.
pub trait Node: Send + Sync {
    /// Stable node-type id, e.g. `"octans.std.threshold"`.
    fn type_id(&self) -> &'static str;
    fn inputs(&self) -> Vec<PortSpec>;
    fn outputs(&self) -> Vec<PortSpec>;
    fn process(&self, inputs: &Inputs, outputs: &mut Outputs);
}
