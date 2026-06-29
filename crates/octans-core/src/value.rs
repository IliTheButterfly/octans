//! The value & type model.
//!
//! Decided design: values flow through the live graph **type-erased** behind an `Arc`
//! (zero-copy sharing is the default; an actual copy is a separate, explicit operation),
//! while ports carry a **stable, named [`TypeSpec`]** — deliberately *not* `std::any::TypeId`,
//! which is non-serializable and build-unstable. The release/codegen path will later
//! monomorphize these erased handles back to concrete types.

use std::any::Any;
use std::sync::Arc;

/// A stable, serializable, human-named type identifier, e.g. `"octans.std.image"`.
///
/// A `&'static str` for v0; this is the slot that becomes a namespaced, plugin-registrable
/// id. The point is only that it is *not* `std::any::TypeId`.
pub type TypeId = &'static str;

/// What a port carries: a base type plus a shape. The shape separates *what* from *how many*
/// — the `Vector` case is the natural unit of data-parallelism (map/batch).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeSpec {
    pub id: TypeId,
    pub shape: Shape,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Shape {
    Scalar,
    /// `None` = length unknown at authoring time.
    Vector(Option<usize>),
}

impl TypeSpec {
    pub const fn scalar(id: TypeId) -> Self {
        Self { id, shape: Shape::Scalar }
    }
    pub const fn vector(id: TypeId, len: Option<usize>) -> Self {
        Self { id, shape: Shape::Vector(len) }
    }
}

/// A type-erased, cheaply-clonable value handle.
///
/// Cloning a `Value` clones an `Arc` — it shares the underlying buffer, never deep-copies.
/// This is how "zero-copy by default" is enforced mechanically: to actually duplicate data a
/// node must deliberately construct a new value.
#[derive(Clone)]
pub struct Value(Arc<dyn Any + Send + Sync>);

impl Value {
    pub fn new<T: Any + Send + Sync>(v: T) -> Self {
        Value(Arc::new(v))
    }

    pub fn downcast_ref<T: Any>(&self) -> Option<&T> {
        self.0.downcast_ref::<T>()
    }

    /// Strong-reference count — used in tests to demonstrate that handing one frame to several
    /// consumers shares the buffer rather than copying it.
    pub fn ref_count(&self) -> usize {
        Arc::strong_count(&self.0)
    }
}
