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

/// Serializes a type-erased value to JSON (returns `None` if the value isn't of this type).
pub type Serializer = fn(&Value) -> Option<serde_json::Value>;

/// Reconstructs a type-erased value from JSON (returns `None` on a shape/type mismatch).
pub type Deserializer = fn(&serde_json::Value) -> Option<Value>;

/// A ready-made [`Comparator`] for any `T: Any + PartialEq` (downcast both, compare).
pub fn eq_via<T: Any + PartialEq>(a: &Value, b: &Value) -> bool {
    match (a.downcast_ref::<T>(), b.downcast_ref::<T>()) {
        (Some(x), Some(y)) => x == y,
        _ => false,
    }
}

/// A ready-made [`Serializer`] for any `T: Any + Serialize`.
pub fn ser_via<T: Any + serde::Serialize>(v: &Value) -> Option<serde_json::Value> {
    v.downcast_ref::<T>()
        .and_then(|x| serde_json::to_value(x).ok())
}

/// A ready-made [`Deserializer`] for any `T: Any + Send + Sync + DeserializeOwned`.
pub fn de_via<T: Any + Send + Sync + serde::de::DeserializeOwned>(
    j: &serde_json::Value,
) -> Option<Value> {
    serde_json::from_value::<T>(j.clone()).ok().map(Value::new)
}

#[derive(Clone)]
pub struct TypeDescriptor {
    pub id: TypeId,
    pub name: &'static str,
    /// Optional equality used to verify strategy variants produce equal outputs.
    pub eq: Option<Comparator>,
    /// Optional JSON (de)serialization, enabling record/replay of this type through file nodes.
    pub ser: Option<Serializer>,
    pub de: Option<Deserializer>,
    // Future: conversions (incl. Upload/Download), viewers.
}

impl TypeDescriptor {
    pub fn new(id: TypeId, name: &'static str) -> Self {
        Self {
            id,
            name,
            eq: None,
            ser: None,
            de: None,
        }
    }

    pub fn with_eq(mut self, eq: Comparator) -> Self {
        self.eq = Some(eq);
        self
    }

    /// Attach JSON (de)serialization — typically `with_serde(ser_via::<T>, de_via::<T>)`.
    pub fn with_serde(mut self, ser: Serializer, de: Deserializer) -> Self {
        self.ser = Some(ser);
        self.de = Some(de);
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

    /// The JSON serializer registered for `id`, if any.
    pub fn serializer(&self, id: TypeId) -> Option<Serializer> {
        self.types.get(id).and_then(|d| d.ser)
    }

    /// The JSON deserializer registered for `id`, if any.
    pub fn deserializer(&self, id: TypeId) -> Option<Deserializer> {
        self.types.get(id).and_then(|d| d.de)
    }
}
