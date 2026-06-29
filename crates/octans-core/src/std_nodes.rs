//! Standard nodes + domain types for the first vertical slice — all pure Rust, no OpenCV.
//!
//! These will eventually move to an `octans-nodes` crate, and `Image` capture/threshold/blob
//! ops will gain OpenCV (CPU) and wgpu (GPU) variants behind strategy groups. For v0 they
//! exist to prove the engine spine end-to-end with zero external dependencies.

use crate::node::{Inputs, Node, Outputs, PortSpec};
use crate::registry::{Registry, TypeDescriptor};
use crate::value::TypeSpec;

// ---- stable, named type ids ----
pub const T_IMAGE: &str = "octans.std.image";
pub const T_U32: &str = "octans.std.u32";

/// A grayscale `u8` image. Shared cheaply via the enclosing `Value`'s `Arc`; a fresh `px`
/// buffer is allocated only when a node *computes* a new image (not for transport).
#[derive(Clone)]
pub struct Image {
    pub w: usize,
    pub h: usize,
    pub px: Vec<u8>,
}

/// Register the slice's domain types into a [`Registry`].
pub fn register_std_types(reg: &mut Registry) {
    reg.register_type(TypeDescriptor { id: T_IMAGE, name: "Image (grayscale, u8)" });
    reg.register_type(TypeDescriptor { id: T_U32, name: "u32" });
}

/// A source: emits a frame containing known bright disks on a dim background.
///
/// `blobs` are `(cx, cy, r)`. Having a *known* number of disks lets the slice assert
/// correctness, not just "it ran".
pub struct SyntheticCamera {
    pub w: usize,
    pub h: usize,
    pub blobs: Vec<(i32, i32, i32)>,
}

impl Node for SyntheticCamera {
    fn type_id(&self) -> &'static str {
        "octans.std.synthetic_camera"
    }
    fn inputs(&self) -> Vec<PortSpec> {
        vec![]
    }
    fn outputs(&self) -> Vec<PortSpec> {
        vec![PortSpec::new("frame", TypeSpec::scalar(T_IMAGE))]
    }
    fn process(&self, _inputs: &Inputs, outputs: &mut Outputs) {
        let mut px = vec![30u8; self.w * self.h]; // dim background
        for &(cx, cy, r) in &self.blobs {
            let r2 = r * r;
            for y in (cy - r).max(0)..(cy + r + 1).min(self.h as i32) {
                for x in (cx - r).max(0)..(cx + r + 1).min(self.w as i32) {
                    let (dx, dy) = (x - cx, y - cy);
                    if dx * dx + dy * dy <= r2 {
                        px[y as usize * self.w + x as usize] = 220; // bright disk
                    }
                }
            }
        }
        outputs.set("frame", Image { w: self.w, h: self.h, px });
    }
}

/// Threshold an image into a binary (`0`/`255`) mask.
///
/// `thr` is a node field for now; it will become a **writable parameter port** so an optimizer
/// node (driven by a portal carrying downstream blob stats) can tune it per camera.
pub struct Threshold {
    pub thr: u8,
}

impl Node for Threshold {
    fn type_id(&self) -> &'static str {
        "octans.std.threshold"
    }
    fn inputs(&self) -> Vec<PortSpec> {
        vec![PortSpec::new("image", TypeSpec::scalar(T_IMAGE))]
    }
    fn outputs(&self) -> Vec<PortSpec> {
        vec![PortSpec::new("mask", TypeSpec::scalar(T_IMAGE))]
    }
    fn process(&self, inputs: &Inputs, outputs: &mut Outputs) {
        let img: &Image = inputs.get("image");
        let px = img
            .px
            .iter()
            .map(|&p| if p >= self.thr { 255 } else { 0 })
            .collect();
        outputs.set("mask", Image { w: img.w, h: img.h, px });
    }
}

/// Count connected components of value `255` (4-connectivity flood fill).
pub struct BlobCount;

impl Node for BlobCount {
    fn type_id(&self) -> &'static str {
        "octans.std.blob_count"
    }
    fn inputs(&self) -> Vec<PortSpec> {
        vec![PortSpec::new("mask", TypeSpec::scalar(T_IMAGE))]
    }
    fn outputs(&self) -> Vec<PortSpec> {
        vec![PortSpec::new("count", TypeSpec::scalar(T_U32))]
    }
    fn process(&self, inputs: &Inputs, outputs: &mut Outputs) {
        let m: &Image = inputs.get("mask");
        let (w, h) = (m.w, m.h);
        let mut seen = vec![false; w * h];
        let mut stack: Vec<usize> = Vec::new();
        let mut count: u32 = 0;

        for start in 0..w * h {
            if m.px[start] != 255 || seen[start] {
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
                    if m.px[ni] == 255 && !seen[ni] {
                        seen[ni] = true;
                        stack.push(ni);
                    }
                }
            }
        }
        outputs.set("count", count);
    }
}

/// A sink: prints the blob count. (Later: a viewer / a boundary output of the embedded graph.)
pub struct Report {
    pub label: &'static str,
}

impl Node for Report {
    fn type_id(&self) -> &'static str {
        "octans.std.report"
    }
    fn inputs(&self) -> Vec<PortSpec> {
        vec![PortSpec::new("count", TypeSpec::scalar(T_U32))]
    }
    fn outputs(&self) -> Vec<PortSpec> {
        vec![]
    }
    fn process(&self, inputs: &Inputs, _outputs: &mut Outputs) {
        let c: &u32 = inputs.get("count");
        eprintln!("[{}] blobs detected: {}", self.label, c);
    }
}
