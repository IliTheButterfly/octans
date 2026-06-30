//! Tracking domain nodes — the concrete use case Octans was born for, now expressed *in* the
//! engine. `Triangulate` recovers a 3D point from N cameras' 2D observations via the Direct
//! Linear Transform (pure Rust, nalgebra SVD — no OpenCV).

use crate::Image;
use nalgebra::{DMatrix, Matrix3x4};
use octans_core::{
    de_via, eq_via, ser_via, Context, Inputs, Node, Outputs, PortSpec, RegisteredType, Registry,
    Shape, TypeDescriptor, TypeId, TypeSpec, Value,
};
use octans_macros::node;
use serde::{Deserialize, Serialize};
use std::any::Any;

/// A 3×4 camera projection matrix `P` (so a homogeneous world point `X` projects to `P·X`).
#[derive(Clone, PartialEq, Debug)]
pub struct Proj(pub Matrix3x4<f64>);

/// A 2D image observation (normalized/undistorted coords).
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct Px(pub [f64; 2]);

/// A 3D point.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct Pt3(pub [f64; 3]);

impl Proj {
    /// A pinhole camera at world position `c` looking down +z (P = [I | -c]).
    pub fn camera(c: [f64; 3]) -> Self {
        Proj(Matrix3x4::new(
            1.0, 0.0, 0.0, -c[0], //
            0.0, 1.0, 0.0, -c[1], //
            0.0, 0.0, 1.0, -c[2],
        ))
    }
}

impl RegisteredType for Proj {
    const ID: TypeId = "octans.track.proj";
}
impl RegisteredType for Px {
    const ID: TypeId = "octans.track.px";
}
impl RegisteredType for Pt3 {
    const ID: TypeId = "octans.track.pt3";
}

/// Register the tracking domain types (with comparators) into a [`Registry`].
pub fn register_tracking_types(reg: &mut Registry) {
    // Proj wraps a nalgebra matrix (no serde without that feature) — comparator only.
    reg.register_type(TypeDescriptor::new(Proj::ID, "projection 3x4").with_eq(eq_via::<Proj>));
    reg.register_type(
        TypeDescriptor::new(Px::ID, "pixel 2d")
            .with_eq(eq_via::<Px>)
            .with_serde(ser_via::<Px>, de_via::<Px>),
    );
    reg.register_type(
        TypeDescriptor::new(Pt3::ID, "point 3d")
            .with_eq(eq_via::<Pt3>)
            .with_serde(ser_via::<Pt3>, de_via::<Pt3>),
    );
}

/// N-view triangulation (DLT): inputs are a `Vector<Proj>` and the matching `Vector<Px>`;
/// the output is the recovered 3D `point`.
pub struct Triangulate;

impl Node for Triangulate {
    fn node_type(&self) -> &'static str {
        "octans.track.triangulate"
    }
    fn inputs(&self) -> Vec<PortSpec> {
        vec![
            PortSpec::new(
                "proj",
                TypeSpec {
                    id: Proj::ID,
                    shape: Shape::Vector(None),
                },
            ),
            PortSpec::new(
                "px",
                TypeSpec {
                    id: Px::ID,
                    shape: Shape::Vector(None),
                },
            ),
        ]
    }
    fn outputs(&self) -> Vec<PortSpec> {
        vec![PortSpec::new("point", Pt3::type_spec())]
    }
    fn process(
        &self,
        _ctx: &Context,
        _local: &mut dyn Any,
        inputs: &Inputs,
        outputs: &mut Outputs,
    ) {
        let projs = inputs.get::<Vec<Value>>("proj");
        let pxs = inputs.get::<Vec<Value>>("px");
        let n = projs.len().min(pxs.len());

        // Keep only finite correspondences — a NaN/Inf observation (or projection) would poison
        // the whole solve, so drop it rather than emit garbage.
        let mut views: Vec<(&Matrix3x4<f64>, f64, f64)> = Vec::with_capacity(n);
        for i in 0..n {
            let p = &projs[i].downcast_ref::<Proj>().expect("proj value").0;
            let px = pxs[i].downcast_ref::<Px>().expect("px value").0;
            let (u, v) = (px[0], px[1]);
            if u.is_finite() && v.is_finite() && p.iter().all(|x| x.is_finite()) {
                views.push((p, u, v));
            }
        }

        // A 3D point is under-determined by fewer than two views: produce nothing (the consumer
        // skips this tick) rather than triangulating noise.
        if views.len() < 2 {
            return;
        }

        // Each view contributes two rows: x×(P·X)=0  ⇒  u·P[2] - P[0], v·P[2] - P[1].
        let mut a = DMatrix::<f64>::zeros(2 * views.len(), 4);
        for (i, (p, u, v)) in views.iter().enumerate() {
            for k in 0..4 {
                a[(2 * i, k)] = u * p[(2, k)] - p[(0, k)];
                a[(2 * i + 1, k)] = v * p[(2, k)] - p[(1, k)];
            }
        }

        // Solution = right singular vector of the smallest singular value (last row of Vᵀ).
        let Some(vt) = a.svd(false, true).v_t else {
            return;
        };
        let row = vt.row(vt.nrows() - 1);
        let w = row[3];
        // A homogeneous w near zero means the cameras are degenerate (e.g. all collinear with
        // the point) — the de-homogenized point would blow up. Reject it, and any non-finite.
        if w.abs() < 1e-9 {
            return;
        }
        let point = [row[0] / w, row[1] / w, row[2] / w];
        if point.iter().all(|c| c.is_finite()) {
            outputs.set("point", Pt3(point));
        }
    }
}

// ---------------------------------------------------------------------------
// A synthetic end-to-end scene: a moving point, pinhole cameras, blob detection.
// ---------------------------------------------------------------------------

/// Ground-truth 3D point that drifts linearly with the tick: `start + vel * tick`.
pub struct MovingPoint {
    pub start: [f64; 3],
    pub vel: [f64; 3],
}

#[node(id = "octans.track.moving_point", out = "point")]
impl MovingPoint {
    fn process(&self, #[ctx] ctx: &Context) -> Pt3 {
        let t = ctx.tick() as f64;
        Pt3([
            self.start[0] + self.vel[0] * t,
            self.start[1] + self.vel[1] * t,
            self.start[2] + self.vel[2] * t,
        ])
    }
}

/// A pinhole camera at world position `center` (looking +z): renders a bright disk where the
/// input 3D point projects. Pixel = `(u·f + w/2, v·f + h/2)` with `(u,v)` the normalized coords.
/// Pair with `Proj::camera(center)` when triangulating.
pub struct CameraSim {
    pub center: [f64; 3],
    pub w: usize,
    pub h: usize,
    pub f: f64,
}

#[node(id = "octans.track.camera_sim", out = "frame")]
impl CameraSim {
    fn process(&self, point: &Pt3) -> Image {
        let mut px = vec![30u8; self.w * self.h]; // dim background
        let rel = [
            point.0[0] - self.center[0],
            point.0[1] - self.center[1],
            point.0[2] - self.center[2],
        ];
        if rel[2] > 0.0 {
            let cx = (rel[0] / rel[2] * self.f + self.w as f64 / 2.0).round() as i32;
            let cy = (rel[1] / rel[2] * self.f + self.h as f64 / 2.0).round() as i32;
            let r = 6i32;
            for y in (cy - r).max(0)..(cy + r + 1).min(self.h as i32) {
                for x in (cx - r).max(0)..(cx + r + 1).min(self.w as i32) {
                    let (dx, dy) = (x - cx, y - cy);
                    if dx * dx + dy * dy <= r * r {
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

/// Find the centroid of the bright (`255`) pixels in a mask and return it as a normalized image
/// observation `(u, v) = ((cx - w/2)/f, (cy - h/2)/f)` — ready to triangulate. Returns `None`
/// when the mask is empty (the target isn't visible this frame), so the observation is simply
/// absent rather than a bogus `(0, 0)` that would corrupt triangulation.
pub struct Centroid {
    pub w: usize,
    pub h: usize,
    pub f: f64,
}

#[node(id = "octans.track.centroid", out = "px")]
impl Centroid {
    fn process(&self, mask: &Image) -> Option<Px> {
        let (mut sx, mut sy, mut n) = (0.0f64, 0.0f64, 0.0f64);
        for y in 0..self.h {
            for x in 0..self.w {
                if mask.px[y * self.w + x] == 255 {
                    sx += x as f64;
                    sy += y as f64;
                    n += 1.0;
                }
            }
        }
        if n == 0.0 {
            return None; // nothing detected this frame
        }
        Some(Px([
            (sx / n - self.w as f64 / 2.0) / self.f,
            (sy / n - self.h as f64 / 2.0) / self.f,
        ]))
    }
}

/// A fused threshold+centroid: in one pass over the frame, take the centroid of all pixels at or
/// above `thr` and return it as a normalized observation. Computes exactly the same `Px` as
/// `Threshold` (default `128`) → `Centroid` — same pixel set, same accumulation order — but in a
/// single pass with no intermediate mask allocation. A drop-in, bit-identical, faster variant: an
/// honest choice for a `Strategy`/autotuner to pick (its boundary `frame → px` matches a
/// `Threshold→Centroid` group's).
pub struct ThresholdCentroid {
    pub w: usize,
    pub h: usize,
    pub f: f64,
    pub thr: u8,
}

#[node(id = "octans.track.threshold_centroid", out = "px")]
impl ThresholdCentroid {
    fn process(&self, frame: &Image) -> Option<Px> {
        let (mut sx, mut sy, mut n) = (0.0f64, 0.0f64, 0.0f64);
        for y in 0..self.h {
            for x in 0..self.w {
                if frame.px[y * self.w + x] >= self.thr {
                    sx += x as f64;
                    sy += y as f64;
                    n += 1.0;
                }
            }
        }
        if n == 0.0 {
            return None;
        }
        Some(Px([
            (sx / n - self.w as f64 / 2.0) / self.f,
            (sy / n - self.h as f64 / 2.0) / self.f,
        ]))
    }
}
