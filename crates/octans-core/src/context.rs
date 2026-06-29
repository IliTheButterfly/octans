//! The global context — shared, read-mostly state available to every node each tick.
//!
//! Holds the tick (frame) counter and a type-keyed **resource map**: data loaded once and
//! shared read-only by all nodes (calibration tables, a GPU device handle, config). Access is
//! `&Context`, so reads parallelize freely. Shared *mutable* global state, if ever needed, is
//! an explicitly-synchronized resource a node opts into — never implicit.

use std::any::{Any, TypeId};
use std::collections::HashMap;

#[derive(Default)]
pub struct Context {
    tick: u64,
    resources: HashMap<TypeId, Box<dyn Any + Send + Sync>>,
}

impl Context {
    pub fn new() -> Self {
        Self::default()
    }

    /// Monotonic tick/frame counter. Starts at 0; the interpreter advances it each tick (so the
    /// first tick a node sees is 1).
    pub fn tick(&self) -> u64 {
        self.tick
    }

    pub(crate) fn advance(&mut self) {
        self.tick += 1;
    }

    /// Insert a shared resource (loaded once, read by nodes via [`resource`](Self::resource)).
    pub fn insert_resource<T: Any + Send + Sync>(&mut self, value: T) {
        self.resources.insert(TypeId::of::<T>(), Box::new(value));
    }

    /// Borrow a shared resource by type, if present.
    pub fn resource<T: Any + Send + Sync>(&self) -> Option<&T> {
        self.resources
            .get(&TypeId::of::<T>())
            .and_then(|b| b.downcast_ref::<T>())
    }
}
