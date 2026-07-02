//! Vector math + compositor image nodes.

use octans_core::*;
use octans_nodes::*;

fn vec_graph(op: &str, a: [f64; 3], b: [f64; 3], s: f64) -> [f64; 3] {
    let mut reg = Registry::new();
    register_primitives(&mut reg);
    register_tracking_types(&mut reg);
    let mut g = Graph::new(reg);
    let m = g.add(VectorMath { op: op.into() });
    let (va, vb, vs) = (
        g.add(PtSource(a)),
        g.add(PtSource(b)),
        g.add(FloatValue { value: s }),
    );
    g.connect(va, "out", m, "a").unwrap();
    g.connect(vb, "out", m, "b").unwrap();
    g.connect(vs, "value", m, "s").unwrap();
    let mut e = Mira::compile(&g).unwrap();
    e.run_tick(&g)
        .output(m, "vector")
        .unwrap()
        .downcast_ref::<Pt3>()
        .unwrap()
        .0
}

#[test]
fn vector_math_ops() {
    assert_eq!(
        vec_graph("cross", [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], 1.0),
        [0.0, 0.0, 1.0]
    );
    assert_eq!(
        vec_graph("scale", [1.0, -2.0, 3.0], [0.0; 3], 2.0),
        [2.0, -4.0, 6.0]
    );
    assert_eq!(
        vec_graph("normalize", [3.0, 0.0, 4.0], [0.0; 3], 1.0),
        [0.6, 0.0, 0.8]
    );
    assert_eq!(
        vec_graph("lerp", [0.0, 0.0, 0.0], [10.0, 20.0, 30.0], 0.5),
        [5.0, 10.0, 15.0]
    );
}

#[test]
fn combine_separate_round_trip_and_reduce() {
    let mut reg = Registry::new();
    register_primitives(&mut reg);
    register_tracking_types(&mut reg);
    let mut g = Graph::new(reg);
    let (x, y, z) = (
        g.add(FloatValue { value: 3.0 }),
        g.add(FloatValue { value: 4.0 }),
        g.add(FloatValue { value: 12.0 }),
    );
    let comb = g.add(CombineXYZ);
    g.connect(x, "value", comb, "x").unwrap();
    g.connect(y, "value", comb, "y").unwrap();
    g.connect(z, "value", comb, "z").unwrap();
    let sep = g.add(SeparateXYZ);
    g.connect(comb, "vector", sep, "vector").unwrap();
    let red = g.add(VectorReduce {
        op: "length".into(),
    });
    g.connect(comb, "vector", red, "a").unwrap();

    let mut e = Mira::compile(&g).unwrap();
    let t = e.run_tick(&g);
    assert_eq!(
        t.output(sep, "y").unwrap().downcast_ref::<f64>(),
        Some(&4.0)
    );
    assert_eq!(
        t.output(red, "value").unwrap().downcast_ref::<f64>(),
        Some(&13.0),
        "|(3,4,12)| = 13"
    );
}

#[test]
fn image_ops_pipeline() {
    let mut reg = Registry::new();
    register_primitives(&mut reg);
    register_node_types(&mut reg);
    let mut g = Graph::new(reg);
    let cam = g.add(SyntheticCamera {
        w: 16,
        h: 16,
        blobs: vec![(8, 8, 3)],
    });
    let inv = g.add(Invert);
    let blur = g.add(BoxBlur { radius: 1 });
    let mix = g.add(MixImages);
    let im = g.add(ImageMath {
        op: "difference".into(),
    });
    g.connect(cam, "frame", inv, "image").unwrap();
    g.connect(cam, "frame", blur, "image").unwrap();
    g.connect(cam, "frame", mix, "a").unwrap();
    g.connect(inv, "image", mix, "b").unwrap();
    g.connect(cam, "frame", im, "a").unwrap();
    g.connect(cam, "frame", im, "b").unwrap();

    let mut e = Mira::compile(&g).unwrap();
    let t = e.run_tick(&g);
    let get = |n, p| {
        t.output(n, p)
            .unwrap()
            .downcast_ref::<Image>()
            .unwrap()
            .clone()
    };

    let src = get(cam, "frame");
    let invd = get(inv, "image");
    assert_eq!(invd.px[0], 255 - src.px[0], "invert");
    let blurred = get(blur, "image");
    assert_eq!(blurred.w, 16);
    // mix at fac 0.5 of image and its inverse ≈ mid-gray everywhere
    let mixed = get(mix, "image");
    assert!(mixed.px.iter().all(|&p| (126..=129).contains(&p)));
    // difference with itself = 0
    let diff = get(im, "image");
    assert!(diff.px.iter().all(|&p| p == 0));
}

// A tiny Pt3 source for the vector tests.
use octans_core::{Context, Inputs, Node, Outputs, PortSpec};
use std::any::Any;
struct PtSource([f64; 3]);
impl Node for PtSource {
    fn node_type(&self) -> &'static str {
        "test.pt_source"
    }
    fn inputs(&self) -> Vec<PortSpec> {
        vec![]
    }
    fn outputs(&self) -> Vec<PortSpec> {
        vec![PortSpec::new("out", Pt3::type_spec())]
    }
    fn process(&self, _c: &Context, _l: &mut dyn Any, _i: &Inputs, o: &mut Outputs) {
        o.set("out", Pt3(self.0));
    }
}
