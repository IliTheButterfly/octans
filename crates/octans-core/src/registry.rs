//! The open type registry.
//!
//! Plugins register their own types here (and conversions + viewers later). A type descriptor
//! carries an optional **comparator** — used by the autotuner's verify-by-default to confirm
//! that interchangeable strategy variants actually produce equal outputs.

use crate::value::{TypeId, Value};
use std::any::Any;
use std::collections::HashMap;

/// Compares two type-erased values of the same registered type.
pub type Comparator = fn(&Value, &Value) -> bool;

/// A ready-made [`Comparator`] for any `T: Any + PartialEq` (downcast both, compare).
pub fn eq_via<T: Any + PartialEq>(a: &Value, b: &Value) -> bool {
    match (a.downcast_ref::<T>(), b.downcast_ref::<T>()) {
        (Some(x), Some(y)) => x == y,
        _ => false,
    }
}

#[derive(Clone)]
pub struct TypeDescriptor {
    pub id: TypeId,
    pub name: &'static str,
    /// Optional equality used to verify strategy variants produce equal outputs.
    pub eq: Option<Comparator>,
    // Future: conversions (incl. Upload/Download), viewers, (de)serializers.
}

impl TypeDescriptor {
    pub fn new(id: TypeId, name: &'static str) -> Self {
        Self { id, name, eq: None }
    }

    pub fn with_eq(mut self, eq: Comparator) -> Self {
        self.eq = Some(eq);
        self
    }
}

#[derive(Default)]
pub struct Registry {
    types: HashMap<TypeId, TypeDescriptor>,
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register_type(&mut self, desc: TypeDescriptor) {
        self.types.insert(desc.id, desc);
    }

    pub fn is_registered(&self, id: TypeId) -> bool {
        self.types.contains_key(id)
    }

    pub fn get(&self, id: TypeId) -> Option<&TypeDescriptor> {
        self.types.get(id)
    }

    /// The comparator registered for `id`, if any.
    pub fn comparator(&self, id: TypeId) -> Option<Comparator> {
        self.types.get(id).and_then(|d| d.eq)
    }
}
