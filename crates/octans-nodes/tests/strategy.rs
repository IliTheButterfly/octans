//! A `Strategy` node holds interchangeable variants and runs the selected one. Switching the
//! handle changes which implementation runs — same boundary, different behaviour/cost.

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

struct AddTen;
#[node(id = "test.add_ten", out = "out")]
impl AddTen {
    fn process(&self, x: &u32) -> u32 {
        *x + 10
    }
}

#[test]
fn strategy_runs_selected_variant() {
    // Two equivalent-signature variants (u32 "x" -> u32 "out"), different behaviour.
    let strat = Strategy::builder()
        .node("add_one", AddOne)
        .node("add_ten", AddTen)
        .build();
    let h = strat.handle();
    assert_eq!(h.variant_count(), 2);

    let mut reg = Registry::new();
    register_primitives(&mut reg);
    let mut g = Graph::new(reg);

    let c = g.add(Const { v: 5 });
    let s = g.add(strat);
    g.connect(c, "out", s, "x").unwrap();

    let mut engine = Mira::compile(&g).unwrap();

    let run = |engine: &mut Mira, g: &Graph| {
        *engine
            .run_tick(g)
            .output(s, "out")
            .unwrap()
            .downcast_ref::<u32>()
            .unwrap()
    };

    h.select_by_name("add_one");
    assert_eq!(run(&mut engine, &g), 6);

    h.select_by_name("add_ten");
    assert_eq!(run(&mut engine, &g), 15); // same graph + engine, different variant now active
}
