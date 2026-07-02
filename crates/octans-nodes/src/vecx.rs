//! Vector (Pt3) math, modeled on Blender's Vector Math node — split into a Pt3→Pt3 node and a
//! Pt3→scalar reduce node (ports are statically typed, so ops can't change the output type), plus
//! Combine/Separate XYZ.

use crate::Pt3;
use octans_core::{
    Catalog, Context, Inputs, Node, NodeRegistry, Outputs, PortSpec, RegisteredType,
};
use octans_macros::{node, NodeParams};
use serde::{Deserialize, Serialize};
use std::any::Any;

/// Vector → vector math on `a`, `b` (`scale`/`lerp` also use the scalar `s`).
#[derive(Serialize, Deserialize, NodeParams)]
pub struct VectorMath {
    /// The operation.
    #[param(
        options = "add,subtract,multiply,divide,cross,scale,normalize,negate,min,max,abs,floor,ceil,fract,lerp"
    )]
    pub op: String,
}

#[node(id = "octans.vector.math", out = "vector", serde, params)]
impl VectorMath {
    fn process(
        &self,
        #[param(default = Pt3([0.0, 0.0, 0.0]))] a: &Pt3,
        #[param(default = Pt3([0.0, 0.0, 0.0]))] b: &Pt3,
        #[param(default = 1.0f64)] s: &f64,
    ) -> Pt3 {
        let (a, b, s) = (a.0, b.0, *s);
        let map2 = |f: fn(f64, f64) -> f64| Pt3([f(a[0], b[0]), f(a[1], b[1]), f(a[2], b[2])]);
        let map1 = |f: fn(f64) -> f64| Pt3([f(a[0]), f(a[1]), f(a[2])]);
        match self.op.as_str() {
            "add" => map2(|x, y| x + y),
            "subtract" => map2(|x, y| x - y),
            "multiply" => map2(|x, y| x * y),
            "divide" => map2(|x, y| if y != 0.0 { x / y } else { 0.0 }),
            "cross" => Pt3([
                a[1] * b[2] - a[2] * b[1],
                a[2] * b[0] - a[0] * b[2],
                a[0] * b[1] - a[1] * b[0],
            ]),
            "scale" => Pt3([a[0] * s, a[1] * s, a[2] * s]),
            "normalize" => {
                let len = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt();
                if len > 0.0 {
                    Pt3([a[0] / len, a[1] / len, a[2] / len])
                } else {
                    Pt3([0.0; 3])
                }
            }
            "negate" => map1(|x| -x),
            "min" => map2(f64::min),
            "max" => map2(f64::max),
            "abs" => map1(f64::abs),
            "floor" => map1(f64::floor),
            "ceil" => map1(f64::ceil),
            "fract" => map1(|x| x - x.floor()),
            "lerp" => Pt3([
                a[0] + (b[0] - a[0]) * s,
                a[1] + (b[1] - a[1]) * s,
                a[2] + (b[2] - a[2]) * s,
            ]),
            _ => Pt3([0.0; 3]),
        }
    }
}

/// Vector → scalar reductions.
#[derive(Serialize, Deserialize, NodeParams)]
pub struct VectorReduce {
    /// The reduction (`length` uses only `a`).
    #[param(options = "dot,length,distance")]
    pub op: String,
}

#[node(id = "octans.vector.reduce", out = "value", serde, params)]
impl VectorReduce {
    fn process(
        &self,
        #[param(default = Pt3([0.0, 0.0, 0.0]))] a: &Pt3,
        #[param(default = Pt3([0.0, 0.0, 0.0]))] b: &Pt3,
    ) -> f64 {
        let (a, b) = (a.0, b.0);
        match self.op.as_str() {
            "dot" => a[0] * b[0] + a[1] * b[1] + a[2] * b[2],
            "length" => (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt(),
            "distance" => {
                let d = [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
                (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
            }
            _ => 0.0,
        }
    }
}

/// Build a vector from three scalars.
#[derive(Serialize, Deserialize)]
pub struct CombineXYZ;

#[node(id = "octans.vector.combine_xyz", out = "vector", serde)]
impl CombineXYZ {
    fn process(
        &self,
        #[param(default = 0.0f64)] x: &f64,
        #[param(default = 0.0f64)] y: &f64,
        #[param(default = 0.0f64)] z: &f64,
    ) -> Pt3 {
        Pt3([*x, *y, *z])
    }
}

/// Split a vector into `x`, `y`, `z` scalars (hand-written: the `#[node]` macro is single-output).
#[derive(Serialize, Deserialize)]
pub struct SeparateXYZ;

impl Node for SeparateXYZ {
    fn node_type(&self) -> &'static str {
        "octans.vector.separate_xyz"
    }
    fn inputs(&self) -> Vec<PortSpec> {
        vec![PortSpec::new("vector", Pt3::type_spec())]
    }
    fn outputs(&self) -> Vec<PortSpec> {
        vec![
            PortSpec::new("x", f64::type_spec()),
            PortSpec::new("y", f64::type_spec()),
            PortSpec::new("z", f64::type_spec()),
        ]
    }
    fn to_json(&self) -> serde_json::Value {
        serde_json::json!({})
    }
    fn process(&self, _c: &Context, _l: &mut dyn Any, i: &Inputs, o: &mut Outputs) {
        let v = i.get::<Pt3>("vector").0;
        o.set("x", v[0]);
        o.set("y", v[1]);
        o.set("z", v[2]);
    }
}

pub fn register_vector_factories(reg: &mut NodeRegistry) {
    reg.register_serde::<VectorMath>("octans.vector.math");
    reg.register_serde::<VectorReduce>("octans.vector.reduce");
    reg.register_serde::<CombineXYZ>("octans.vector.combine_xyz");
    reg.register("octans.vector.separate_xyz", |_| Box::new(SeparateXYZ));
}

pub fn register_vector_catalog(cat: &mut Catalog) {
    cat.add(|| VectorMath { op: "add".into() });
    cat.add(|| VectorReduce { op: "dot".into() });
    cat.add(|| CombineXYZ);
    cat.add(|| SeparateXYZ);
}
