//! Data-parallel fan-out via `Map`: apply a unary node to each element of a vector, in
//! parallel, with **independent per-lane state**.

use octans_core::*;
use octans_macros::node;

// A source that emits a fixed `Vector<u32>` (hand-written: the vector type isn't a scalar that
// `#[node]`'s return-type derivation handles yet).
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
    fn process(
        &self,
        _ctx: &Context,
        _local: &mut dyn std::any::Any,
        _in: &Inputs,
        out: &mut Outputs,
    ) {
        out.set_value(
            "items",
            Value::vector(self.items.iter().map(|&x| Value::new(x)).collect()),
        );
    }
}

// Stateless unary node.
struct Increment;
#[node(id = "test.inc", out = "out")]
impl Increment {
    fn process(&self, x: &u32) -> u32 {
        *x + 1
    }
}

// Stateful unary node: each lane keeps its own running sum.
#[derive(Default)]
struct Acc {
    sum: u64,
}
struct Accumulate;
#[node(id = "test.acc", out = "sum")]
impl Accumulate {
    fn process(&self, #[local] s: &mut Acc, x: &u32) -> u64 {
        s.sum += *x as u64;
        s.sum
    }
}

fn as_u32s(v: &Value) -> Vec<u32> {
    v.as_vector()
        .unwrap()
        .iter()
        .map(|e| *e.downcast_ref::<u32>().unwrap())
        .collect()
}
fn as_u64s(v: &Value) -> Vec<u64> {
    v.as_vector()
        .unwrap()
        .iter()
        .map(|e| *e.downcast_ref::<u64>().unwrap())
        .collect()
}

#[test]
fn map_applies_inner_per_element() {
    let mut reg = Registry::new();
    register_primitives(&mut reg);
    let mut g = Graph::new(reg);

    let src = g.add(VecSource {
        items: vec![1, 2, 3],
    });
    let m = g.add(Map::new(Increment));
    g.connect(src, "items", m, "items").unwrap();

    let mut engine = Mira::compile(&g).unwrap();
    let tick = engine.run_tick(&g);
    assert_eq!(as_u32s(tick.output(m, "items").unwrap()), vec![2, 3, 4]);
}

#[test]
fn map_gives_each_lane_independent_state() {
    let mut reg = Registry::new();
    register_primitives(&mut reg);
    let mut g = Graph::new(reg);

    let src = g.add(VecSource {
        items: vec![10, 20, 30],
    });
    let m = g.add(Map::new(Accumulate));
    g.connect(src, "items", m, "items").unwrap();

    let mut engine = Mira::compile(&g).unwrap();
    let mut last = Vec::new();
    for _ in 0..3 {
        last = as_u64s(engine.run_tick(&g).output(m, "items").unwrap());
    }
    // each lane accumulated ITS element three times, independently
    assert_eq!(last, vec![30, 60, 90]);
}
