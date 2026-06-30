//! Tracking domain nodes — the concrete use case Octans was born for, now expressed *in* the
//! engine. `Triangulate` recovers a 3D point from N cameras' 2D observations via the Direct
//! Linear Transform (pure Rust, nalgebra SVD — no OpenCV).

use nalgebra::{DMatrix, Matrix3x4};
use octans_core::{
    eq_via, Context, Inputs, Node, Outputs, PortSpec, RegisteredType, Registry, Shape,
    TypeDescriptor, TypeId, TypeSpec, Value,
};
use std::any::Any;

/// A 3×4 camera projection matrix `P` (so a homogeneous world point `X` projects to `P·X`).
#[derive(Clone, PartialEq)]
pub struct Proj(pub Matrix3x4<f64>);

/// A 2D image observation (normalized/undistorted coords).
#[derive(Clone, PartialEq)]
pub struct Px(pub [f64; 2]);

/// A 3D point.
#[derive(Clone, PartialEq, Debug)]
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
    reg.register_type(TypeDescriptor::new(Proj::ID, "projection 3x4").with_eq(eq_via::<Proj>));
    reg.register_type(TypeDescriptor::new(Px::ID, "pixel 2d").with_eq(eq_via::<Px>));
    reg.register_type(TypeDescriptor::new(Pt3::ID, "point 3d").with_eq(eq_via::<Pt3>));
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

        // Each view contributes two rows: x×(P·X)=0  ⇒  u·P[2] - P[0], v·P[2] - P[1].
        let mut a = DMatrix::<f64>::zeros(2 * n, 4);
        for i in 0..n {
            let p = &projs[i].downcast_ref::<Proj>().expect("proj value").0;
            let px = pxs[i].downcast_ref::<Px>().expect("px value").0;
            let (u, v) = (px[0], px[1]);
            for k in 0..4 {
                a[(2 * i, k)] = u * p[(2, k)] - p[(0, k)];
                a[(2 * i + 1, k)] = v * p[(2, k)] - p[(1, k)];
            }
        }

        // Solution = right singular vector of the smallest singular value (last row of Vᵀ).
        let svd = a.svd(false, true);
        let vt = svd.v_t.expect("V computed");
        let row = vt.row(vt.nrows() - 1);
        let w = row[3];
        outputs.set("point", Pt3([row[0] / w, row[1] / w, row[2] / w]));
    }
}
