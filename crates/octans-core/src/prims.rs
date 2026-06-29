//! Built-in scalar types known to the engine.
//!
//! Their [`RegisteredType`] impls live here — in the crate that owns the trait — to satisfy
//! Rust's orphan rule (we can't impl our trait for `u32` from another crate).

use crate::registry::{Registry, TypeDescriptor};
use crate::value::{RegisteredType, TypeId};

pub const BOOL: TypeId = "octans.bool";
pub const I32: TypeId = "octans.i32";
pub const U32: TypeId = "octans.u32";
pub const F32: TypeId = "octans.f32";

impl RegisteredType for bool {
    const ID: TypeId = BOOL;
}
impl RegisteredType for i32 {
    const ID: TypeId = I32;
}
impl RegisteredType for u32 {
    const ID: TypeId = U32;
}
impl RegisteredType for f32 {
    const ID: TypeId = F32;
}

/// Register the built-in scalar types into a [`Registry`].
pub fn register_primitives(reg: &mut Registry) {
    reg.register_type(TypeDescriptor { id: BOOL, name: "bool" });
    reg.register_type(TypeDescriptor { id: I32, name: "i32" });
    reg.register_type(TypeDescriptor { id: U32, name: "u32" });
    reg.register_type(TypeDescriptor { id: F32, name: "f32" });
}
