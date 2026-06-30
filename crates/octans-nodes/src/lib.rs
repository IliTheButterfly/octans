//! `octans-nodes` — the standard node library.
//!
//! This is the first real consumer of the `#[node]` macro and proves it works *across a crate
//! boundary* (the actual plugin-author scenario). Each node is a plain struct (its fields are
//! construction-time config) plus an `impl` block with a typed `process` — the macro derives
//! the `Node` impl, the port specs, and the type-erase glue.

use octans_core::{
    Context, Inputs, Node, NodeRegistry, Outputs, PortSpec, RegisteredType, Registry, Shape,
    TypeDescriptor, TypeId, TypeSpec, Value,
};
use octans_macros::node;
use serde::{Deserialize, Serialize};

pub mod tracking;
pub use tracking::*;

/// A source that emits a fixed `Vector<T>` for any registered element type (handy for feeding
/// `Map`/`Triangulate`/etc. from constant data).
pub struct VecConst<T> {
    pub items: Vec<T>,
}

impl<T: RegisteredType + Clone> Node for VecConst<T> {
    fn node_type(&self) -> &'static str {
        "octans.std.vec_const"
    }
    fn inputs(&self) -> Vec<PortSpec> {
        Vec::new()
    }
    fn outputs(&self) -> Vec<PortSpec> {
        vec![PortSpec::new(
            "items",
            TypeSpec {
                id: T::ID,
                shape: Shape::Vector(None),
            },
        )]
    }
    fn process(&self, _c: &Context, _l: &mut dyn std::any::Any, _i: &Inputs, out: &mut Outputs) {
        out.set_value(
            "items",
            Value::vector(self.items.iter().cloned().map(Value::new).collect()),
        );
    }
}

/// Stable id for the grayscale image type.
pub const T_IMAGE: TypeId = "octans.std.image";

/// A grayscale `u8` image. Shared cheaply via the enclosing `Value`'s `Arc`; a fresh `px`
/// buffer is allocated only when a node *computes* a new image (not for transport).
#[derive(Clone, PartialEq)]
pub struct Image {
    pub w: usize,
    pub h: usize,
    pub px: Vec<u8>,
}

impl RegisteredType for Image {
    const ID: TypeId = T_IMAGE;
}

/// Register the standard node-library types into a [`Registry`].
/// (Primitives like `u32` are registered separately via `octans_core::register_primitives`.)
pub fn register_node_types(reg: &mut Registry) {
    reg.register_type(
        TypeDescriptor::new(T_IMAGE, "Image (grayscale, u8)").with_eq(octans_core::eq_via::<Image>),
    );
}

/// Register deserialization factories for the serde-able standard nodes, so a `GraphSpec` can be
/// rebuilt into a live graph.
pub fn register_std_factories(reg: &mut NodeRegistry) {
    reg.register_serde::<SyntheticCamera>("octans.std.synthetic_camera");
    reg.register_serde::<Threshold>("octans.std.threshold");
    reg.register_serde::<BlobCount>("octans.std.blob_count");
}

/// A source: emits a frame with known bright disks on a dim background. A *known* blob count
/// lets the slice assert correctness, not just "it ran".
#[derive(Serialize, Deserialize)]
pub struct SyntheticCamera {
    pub w: usize,
    pub h: usize,
    pub blobs: Vec<(i32, i32, i32)>, // (cx, cy, r)
}

#[node(id = "octans.std.synthetic_camera", out = "frame", serde)]
impl SyntheticCamera {
    fn process(&self) -> Image {
        let mut px = vec![30u8; self.w * self.h]; // dim background
        for &(cx, cy, r) in &self.blobs {
            let r2 = r * r;
            for y in (cy - r).max(0)..(cy + r + 1).min(self.h as i32) {
                for x in (cx - r).max(0)..(cx + r + 1).min(self.w as i32) {
                    let (dx, dy) = (x - cx, y - cy);
                    if dx * dx + dy * dy <= r2 {
                        px[y as usize * self.w + x as usize] = 220;
                    }
                }
            }
        }
        Image {
            w: self.w,
            h: self.h,
            px,
        }
    }
}

/// Threshold an image into a binary (`0`/`255`) mask.
///
/// `thr` is a **parameter port** (default `128`): leave it unconnected and it uses the default,
/// or wire an optimizer to it to drive it per camera at runtime.
#[derive(Serialize, Deserialize)]
pub struct Threshold;

#[node(id = "octans.std.threshold", out = "mask", serde)]
impl Threshold {
    fn process(&self, image: &Image, #[param(default = 128u8)] thr: &u8) -> Image {
        let t = *thr;
        let px = image
            .px
            .iter()
            .map(|&p| if p >= t { 255 } else { 0 })
            .collect();
        Image {
            w: image.w,
            h: image.h,
            px,
        }
    }
}

/// Count connected components of value `255` (4-connectivity flood fill).
#[derive(Serialize, Deserialize)]
pub struct BlobCount;

#[node(id = "octans.std.blob_count", out = "count", serde)]
impl BlobCount {
    fn process(&self, mask: &Image) -> u32 {
        let (w, h) = (mask.w, mask.h);
        let mut seen = vec![false; w * h];
        let mut stack: Vec<usize> = Vec::new();
        let mut count: u32 = 0;

        for start in 0..w * h {
            if mask.px[start] != 255 || seen[start] {
                continue;
            }
            count += 1;
            seen[start] = true;
            stack.push(start);
            while let Some(idx) = stack.pop() {
                let (x, y) = (idx % w, idx / w);
                let mut neigh = [None; 4];
                if x > 0 {
                    neigh[0] = Some(idx - 1);
                }
                if x + 1 < w {
                    neigh[1] = Some(idx + 1);
                }
                if y > 0 {
                    neigh[2] = Some(idx - w);
                }
                if y + 1 < h {
                    neigh[3] = Some(idx + w);
                }
                for ni in neigh.into_iter().flatten() {
                    if mask.px[ni] == 255 && !seen[ni] {
                        seen[ni] = true;
                        stack.push(ni);
                    }
                }
            }
        }
        count
    }
}

/// The optimizer's per-instance controller memory: the current threshold. Starts high so the
/// loop visibly hunts downward into range. One of these per lane when the optimizer is fanned
/// out over cameras.
#[derive(Debug)]
pub struct AutoThresholdState {
    pub thr: u8,
}

impl Default for AutoThresholdState {
    fn default() -> Self {
        Self { thr: 255 }
    }
}

/// A proportional optimizer: nudges a threshold toward whatever value yields `target` blobs.
///
/// Its controller memory lives in **local state** (`AutoThresholdState`) — node-local, runtime-
/// owned, replicated per lane. It observes the downstream blob count through a single portal
/// (last tick) and drives `Threshold.thr` this tick. Wire: `BlobCount.count ─portal→ count`,
/// and `thr → Threshold.thr`.
pub struct AutoThreshold {
    pub target: u32,
    pub gain: i32,
    pub min: u8,
    pub max: u8,
}

#[node(id = "octans.std.auto_threshold", out = "thr")]
impl AutoThreshold {
    fn process(&self, #[local] s: &mut AutoThresholdState, count: &u32) -> u8 {
        let err = *count as i32 - self.target as i32; // >0: too many blobs -> raise threshold
        s.thr = (s.thr as i32 + self.gain * err).clamp(self.min as i32, self.max as i32) as u8;
        s.thr
    }
}

/// A sink: prints the blob count. (Later: a viewer / a boundary output of the embedded graph.)
pub struct Report {
    pub label: &'static str,
}

#[node(id = "octans.std.report")]
impl Report {
    fn process(&self, count: &u32) {
        eprintln!("[{}] blobs detected: {}", self.label, count);
    }
}
