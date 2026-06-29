//! The two non-dataflow state tiers: per-instance **local** state, and the shared **context**
//! (tick + resources). Both authored through `#[node]`.

use octans_core::*;
use octans_macros::node;

// ---------- local state: a counter with no inputs ----------

#[derive(Default)]
struct CounterState {
    n: u64,
}

struct Counter;

#[node(id = "test.counter", out = "n")]
impl Counter {
    fn process(&self, #[local] s: &mut CounterState) -> u64 {
        s.n += 1;
        s.n
    }
}

#[test]
fn local_state_persists_across_ticks() {
    let mut reg = Registry::new();
    register_primitives(&mut reg);
    let mut g = Graph::new(reg);
    let c = g.add(Counter);

    let mut engine = Mira::compile(&g).unwrap();
    let mut seen = Vec::new();
    for _ in 0..4 {
        let t = engine.run_tick(&g);
        seen.push(
            t.output(c, "n")
                .and_then(|v| v.downcast_ref::<u64>())
                .copied()
                .unwrap(),
        );
    }
    assert_eq!(
        seen,
        vec![1, 2, 3, 4],
        "local state must persist + evolve per instance"
    );
}

// ---------- global context: tick counter + a shared resource ----------

struct Offset(u64);

struct ClockPlusOffset;

#[node(id = "test.clock_plus_offset", out = "out")]
impl ClockPlusOffset {
    fn process(&self, #[ctx] ctx: &Context) -> u64 {
        ctx.tick() + ctx.resource::<Offset>().map(|o| o.0).unwrap_or(0)
    }
}

#[test]
fn context_exposes_tick_and_shared_resources() {
    let mut reg = Registry::new();
    register_primitives(&mut reg);
    let mut g = Graph::new(reg);
    let n = g.add(ClockPlusOffset);

    let mut engine = Mira::compile(&g).unwrap();
    engine.context_mut().insert_resource(Offset(100)); // loaded once, shared read-only

    let mut seen = Vec::new();
    for _ in 0..3 {
        let t = engine.run_tick(&g);
        seen.push(
            t.output(n, "out")
                .and_then(|v| v.downcast_ref::<u64>())
                .copied()
                .unwrap(),
        );
    }
    // tick advances to 1 on the first tick; + the shared offset of 100.
    assert_eq!(seen, vec![101, 102, 103]);
}
