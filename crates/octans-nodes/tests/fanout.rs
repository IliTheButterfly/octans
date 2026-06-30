//! Finishing fan-out: zip-map (a 2-input body over two vectors) and Gather/Scatter
//! (pack N sources into a vector, unpack a vector to N consumers).

use octans_core::*;
use octans_macros::node;
use std::any::Any;

// ---- helpers: scalar source + vector source ----

struct Const {
    v: u32,
}
#[node(id = "test.const", out = "out")]
impl Const {
    fn process(&self) -> u32 {
        self.v
    }
}

struct VecSource {
    items: Vec<u32>,
}
impl Node for VecSource {
    fn node_type(&self) -> &'static str {
        "test.vec_source"
    }
    fn inputs(&self) -> Vec<PortSpec> {
        Vec::new()
    }
    fn outputs(&self) -> Vec<PortSpec> {
        vec![PortSpec::new(
            "items",
            TypeSpec {
                id: <u32 as RegisteredType>::ID,
                shape: Shape::Vector(None),
            },
        )]
    }
    fn process(&self, _c: &Context, _l: &mut dyn Any, _i: &Inputs, out: &mut Outputs) {
        out.set_value(
            "items",
            Value::vector(self.items.iter().map(|&x| Value::new(x)).collect()),
        );
    }
}

// A 2-input body, to be zipped over two vectors.
struct Add2;
#[node(id = "test.add2", out = "sum")]
impl Add2 {
    fn process(&self, a: &u32, b: &u32) -> u32 {
        *a + *b
    }
}

fn as_u32s(v: &Value) -> Vec<u32> {
    v.as_vector()
        .unwrap()
        .iter()
        .map(|e| *e.downcast_ref::<u32>().unwrap())
        .collect()
}

#[test]
fn zip_map_over_two_vectors() {
    let mut reg = Registry::new();
    register_primitives(&mut reg);
    let mut g = Graph::new(reg);

    let a = g.add(VecSource {
        items: vec![1, 2, 3],
    });
    let b = g.add(VecSource {
        items: vec![10, 20, 30],
    });
    let m = g.add(Map::new(Add2)); // two required inputs "a","b" -> two zipped input ports
    g.connect(a, "items", m, "a").unwrap();
    g.connect(b, "items", m, "b").unwrap();

    let mut engine = Mira::compile(&g).unwrap();
    let tick = engine.run_tick(&g);
    assert_eq!(as_u32s(tick.output(m, "sum").unwrap()), vec![11, 22, 33]);
}

#[test]
fn gather_map_scatter_round_trip() {
    let mut reg = Registry::new();
    register_primitives(&mut reg);
    let mut g = Graph::new(reg);

    // three independent sources -> Gather -> Map(+1) -> Scatter -> three consumers
    let c0 = g.add(Const { v: 10 });
    let c1 = g.add(Const { v: 20 });
    let c2 = g.add(Const { v: 30 });

    let gather = g.add(Gather::new::<u32>(3));
    g.connect(c0, "out", gather, "in0").unwrap();
    g.connect(c1, "out", gather, "in1").unwrap();
    g.connect(c2, "out", gather, "in2").unwrap();

    let m = g.add(Map::new(AddOne));
    g.connect(gather, "items", m, "x").unwrap();

    let scatter = g.add(Scatter::new::<u32>(3));
    g.connect(m, "out", scatter, "items").unwrap();

    let mut engine = Mira::compile(&g).unwrap();
    let tick = engine.run_tick(&g);

    let v0 = *tick
        .output(scatter, "out0")
        .unwrap()
        .downcast_ref::<u32>()
        .unwrap();
    let v1 = *tick
        .output(scatter, "out1")
        .unwrap()
        .downcast_ref::<u32>()
        .unwrap();
    let v2 = *tick
        .output(scatter, "out2")
        .unwrap()
        .downcast_ref::<u32>()
        .unwrap();
    assert_eq!([v0, v1, v2], [11, 21, 31]);
}

struct AddOne;
#[node(id = "test.add_one", out = "out")]
impl AddOne {
    fn process(&self, x: &u32) -> u32 {
        *x + 1
    }
}
