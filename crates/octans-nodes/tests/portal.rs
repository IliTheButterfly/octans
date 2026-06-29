//! Portal semantics: a feedback loop that stays acyclic for scheduling, and carries state
//! across ticks with a one-tick delay (z⁻¹).
//!
//! The classic test: a counter whose *entire* state lives in a portal. `PortalRead → Increment
//! → PortalWrite` is a cycle in intent but a DAG in dataflow (the portal breaks it), so it
//! compiles — and the value climbs 1,2,3,… one step per tick.

use octans_core::*;
use octans_macros::node;

struct Increment;

#[node(id = "test.increment", out = "out")]
impl Increment {
    fn process(&self, n: &u32) -> u32 {
        *n + 1
    }
}

#[test]
fn portal_breaks_the_cycle_and_carries_state() {
    let mut reg = Registry::new();
    register_primitives(&mut reg);
    let mut g = Graph::new(reg);

    let p = g.add_portal(<u32 as RegisteredType>::type_spec(), Value::new(0u32));
    let reader = g.add(p.reader("n"));
    let inc = g.add(Increment);
    let writer = g.add(p.writer("n"));

    g.connect(reader, "n", inc, "n").unwrap();
    g.connect(inc, "out", writer, "n").unwrap();

    // Feedback in intent, DAG in dataflow — must compile.
    let mut engine = Mira::compile(&g).expect("portal breaks the cycle; graph is schedulable");

    let mut seen = Vec::new();
    for _ in 0..5 {
        let tick = engine.run_tick(&g);
        let v = tick
            .output(inc, "out")
            .and_then(|x| x.downcast_ref::<u32>())
            .copied()
            .unwrap();
        seen.push(v);
    }

    assert_eq!(
        seen,
        vec![1, 2, 3, 4, 5],
        "the counter's state lives only in the portal and must climb one step per tick"
    );
}
