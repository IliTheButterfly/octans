//! Built-in scalar types known to the engine.
//!
//! Their [`RegisteredType`] impls live here — in the crate that owns the trait — to satisfy
//! Rust's orphan rule (we can't impl our trait for `u32` from another crate).

use crate::registry::{Registry, TypeDescriptor};
use crate::value::{RegisteredType, TypeId};

macro_rules! prim {
    ($($t:ty => $id:literal),+ $(,)?) => {
        $(
            impl RegisteredType for $t {
                const ID: TypeId = $id;
            }
        )+

        /// Register the built-in scalar types into a [`Registry`] (with `==` comparators and
        /// JSON (de)serialization, so primitives can be recorded/replayed through file nodes).
        pub fn register_primitives(reg: &mut Registry) {
            $(
                reg.register_type(
                    TypeDescriptor::new(<$t as RegisteredType>::ID, stringify!($t))
                        .with_eq(crate::registry::eq_via::<$t>)
                        .with_serde(crate::registry::ser_via::<$t>, crate::registry::de_via::<$t>),
                );
            )+
        }
    };
}

prim! {
    bool => "octans.bool",
    i32  => "octans.i32",
    i64  => "octans.i64",
    u8   => "octans.u8",
    u16  => "octans.u16",
    u32  => "octans.u32",
    u64  => "octans.u64",
    f32  => "octans.f32",
    f64  => "octans.f64",
}
