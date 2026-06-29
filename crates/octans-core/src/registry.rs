//! The open type registry.
//!
//! Plugins register their own types here (and, later, conversions + viewers + serializers).
//! v0 keeps a descriptor with just an id and a human name; the connect-time checker uses it to
//! confirm that every port references a registered type.

use crate::value::TypeId;
use std::collections::HashMap;

#[derive(Clone, Debug)]
pub struct TypeDescriptor {
    pub id: TypeId,
    pub name: &'static str,
    // Future: conversions (incl. Upload/Download), viewers, (de)serializers.
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
}
