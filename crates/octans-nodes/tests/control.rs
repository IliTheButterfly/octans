//! Switch routes one of N inputs; Loop applies a body a fixed number of times.

use octans_core::*;
use octans_macros::node;

struct Const {
    v: u32,
}
#[node(id = "test.const", out = "out")]
impl Const {
    fn process(&self) -> u32 {
        self.v
    }
}

struct AddOne;
#[node(id = "test.add_one", out = "out")]
impl AddOne {
    fn process(&self, x: &u32) -> u32 {
        *x + 1
    }
}

#[test]
fn switch_routes_the_selected_input() {
    let mut reg = Registry::new();
    register_primitives(&mut reg);
    let mut g = Graph::new(reg);

    let a = g.add(Const { v: 10 });
    let b = g.add(Const { v: 20 });
    let c = g.add(Const { v: 30 });
    let sel = g.add(Const { v: 1 });
    let sw = g.add(Switch::new::<u32>(3));
    g.connect(sel, "out", sw, "select").unwrap();
    g.connect(a, "out", sw, "in0").unwrap();
    g.connect(b, "out", sw, "in1").unwrap();
    g.connect(c, "out", sw, "in2").unwrap();

    let mut engine = Mira::compile(&g).unwrap();
    let out = *engine
        .run_tick(&g)
        .output(sw, "out")
        .unwrap()
        .downcast_ref::<u32>()
        .unwrap();
    assert_eq!(out, 20, "select = 1 forwards in1");
}

#[test]
fn loop_applies_body_count_times() {
    let mut reg = Registry::new();
    register_primitives(&mut reg);
    let mut g = Graph::new(reg);

    let c = g.add(Const { v: 0 });
    let lp = g.add(Loop::new(5, AddOne)); // +1, five times
    g.connect(c, "out", lp, "x").unwrap();

    let mut engine = Mira::compile(&g).unwrap();
    let out = *engine
        .run_tick(&g)
        .output(lp, "out")
        .unwrap()
        .downcast_ref::<u32>()
        .unwrap();
    assert_eq!(out, 5, "0 looped through +1 five times");
}
