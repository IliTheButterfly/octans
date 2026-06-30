//! `octans-core` — the engine spine.
//!
//! Octans is a realtime, push-based, visually-authored node engine that compiles to an
//! embeddable, self-optimizing component. This crate is the v0 vertical slice: it holds
//! everything for now, and per the build-order discipline we split out `octans-runtime`,
//! `octans-nodes`, `octans-codegen`, etc. only when real code forces a seam — never as
//! empty scaffolding (the mistake that sank the previous Rust attempt).
//!
//! Load-bearing decisions exercised by the slice:
//! - **Open, named type system** — stable string [`TypeId`]s, not `std::any::TypeId`.
//! - **Type-erased value handles** — [`Value`] is `Arc`-backed; cloning shares, never copies.
//! - **Connect-time type checking** — [`Graph::connect`] rejects mismatches, pointing at ports.
//! - **Compile-once / run-many** — [`Mira`] (the interpreter engine) builds a topological
//!   order, then runs ticks and times them.

pub mod context;
pub mod graph;
pub mod group;
pub mod interp;
pub mod map;
pub mod node;
pub mod portal;
pub mod prims;
pub mod registry;
pub mod value;

pub use context::Context;
pub use graph::{ConnectError, Graph, NodeId};
pub use group::{group, GroupBuilder, GroupInstance, GroupTemplate};
pub use interp::{CompileError, Mira, Tick};
pub use map::Map;
pub use node::{Inputs, Node, Outputs, PortSpec};
pub use portal::{Portal, PortalRead, PortalWrite};
pub use prims::register_primitives;
pub use registry::{Registry, TypeDescriptor};
pub use value::{RegisteredType, Shape, TypeId, TypeSpec, Value};
