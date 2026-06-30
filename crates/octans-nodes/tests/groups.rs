//! Groups flatten into the runtime plan: a group runs identically to its inlined contents,
//! and nesting works (groups inside groups).

use octans_core::*;
use octans_macros::node;
use octans_nodes::*;

#[test]
fn group_flattens_and_runs() {
    // A reusable "detect" group: Threshold -> BlobCount, exposing image->count at its boundary.
    let detect = group("detect", |g| {
        let t = g.add(Threshold);
        let b = g.add(BlobCount);
        g.connect(t, "mask", b, "mask");
        g.input("image", t, "image"); // boundary input (inner face)
        g.output("count", b, "count"); // boundary output (inner face)
    });

    let mut reg = Registry::new();
    register_primitives(&mut reg);
    register_node_types(&mut reg);
    let mut graph = Graph::new(reg);

    let cam = graph.add(SyntheticCamera {
        w: 128,
        h: 128,
        blobs: vec![(30, 30, 8), (90, 40, 10), (60, 100, 6)],
    });
    let inst = graph.add_group(&detect).unwrap();

    // Wire the outer faces: camera -> group.image ; group.count -> (read out)
    let (img_n, img_p) = inst.input("image");
    graph.connect(cam, "frame", img_n, img_p).unwrap();
    let (cnt_n, cnt_p) = inst.output("count");

    let mut engine = Mira::compile(&graph).unwrap();
    let tick = engine.run_tick(&graph);
    let count = tick
        .output(cnt_n, cnt_p)
        .and_then(|v| v.downcast_ref::<u32>())
        .copied()
        .unwrap();
    assert_eq!(count, 3, "the group runs exactly like the inlined pipeline");
}

// ---- nesting ----

struct Const {
    v: u32,
}
#[node(id = "test.const", out = "out")]
impl Const {
    fn process(&self) -> u32 {
        self.v
    }
}

struct Increment;
#[node(id = "test.increment", out = "out")]
impl Increment {
    fn process(&self, x: &u32) -> u32 {
        *x + 1
    }
}

#[test]
fn nested_group_flattens_and_runs() {
    let inc = group("inc", |g| {
        let n = g.add(Increment);
        g.input("x", n, "x");
        g.output("out", n, "out");
    });

    // A group that contains TWO `inc` groups in series -> adds 2.
    let inc2 = group("inc2", move |g| {
        let a = g.add_group(&inc);
        let b = g.add_group(&inc);
        let (ao_n, ao_p) = a.output("out");
        let (bx_n, bx_p) = b.input("x");
        g.connect(ao_n, ao_p, bx_n, bx_p);
        let (ax_n, ax_p) = a.input("x");
        g.input("x", ax_n, ax_p);
        let (bo_n, bo_p) = b.output("out");
        g.output("out", bo_n, bo_p);
    });

    let mut reg = Registry::new();
    register_primitives(&mut reg);
    let mut graph = Graph::new(reg);

    let c = graph.add(Const { v: 5 });
    let inst = graph.add_group(&inc2).unwrap();
    let (xn, xp) = inst.input("x");
    graph.connect(c, "out", xn, xp).unwrap();
    let (on, op) = inst.output("out");

    let mut engine = Mira::compile(&graph).unwrap();
    let tick = engine.run_tick(&graph);
    let out = tick
        .output(on, op)
        .and_then(|v| v.downcast_ref::<u32>())
        .copied()
        .unwrap();
    assert_eq!(out, 7, "5 through two nested +1 groups -> 7");
}
