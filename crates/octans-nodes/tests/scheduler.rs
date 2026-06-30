//! The top-level scheduler runs each depth-level's independent nodes in parallel. A diamond
//! (one source feeding two independent branches that merge) must compute correctly — the two
//! branches sit at the same level and run concurrently.

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

struct TenX;
#[node(id = "test.ten_x", out = "out")]
impl TenX {
    fn process(&self, x: &u32) -> u32 {
        *x * 10
    }
}

struct Add2;
#[node(id = "test.add2", out = "sum")]
impl Add2 {
    fn process(&self, a: &u32, b: &u32) -> u32 {
        *a + *b
    }
}

#[test]
fn diamond_runs_level_parallel_and_is_correct() {
    let mut reg = Registry::new();
    register_primitives(&mut reg);
    let mut g = Graph::new(reg);

    //        ┌── AddOne ──┐
    // Const ─┤            ├── Add2
    //        └── TenX  ───┘
    let c = g.add(Const { v: 5 });
    let a = g.add(AddOne); // level 1
    let b = g.add(TenX); // level 1 (independent of AddOne -> run together)
    let s = g.add(Add2); // level 2
    g.connect(c, "out", a, "x").unwrap();
    g.connect(c, "out", b, "x").unwrap();
    g.connect(a, "out", s, "a").unwrap();
    g.connect(b, "out", s, "b").unwrap();

    let mut engine = Mira::compile(&g).unwrap();
    let tick = engine.run_tick(&g);

    let sum = tick
        .output(s, "sum")
        .and_then(|v| v.downcast_ref::<u32>())
        .copied()
        .unwrap();
    assert_eq!(
        sum,
        (5 + 1) + (5 * 10),
        "diamond merges both branches correctly"
    ); // 6 + 50 = 56

    // profiler saw all four nodes
    assert_eq!(engine.profile().len(), 4);
    for (_n, stat) in engine.profile().iter() {
        assert_eq!(stat.samples, 1);
    }
}
