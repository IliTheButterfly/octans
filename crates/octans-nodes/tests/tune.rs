//! The autotuner benchmarks a Strategy's variants and selects the fastest. Here two variants
//! compute the same thing (`x + 1`), but one does pointless busywork first — the tuner must
//! pick the cheap one.

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

struct Fast;
#[node(id = "test.fast", out = "out")]
impl Fast {
    fn process(&self, x: &u32) -> u32 {
        *x + 1
    }
}

struct Slow;
#[node(id = "test.slow", out = "out")]
impl Slow {
    fn process(&self, x: &u32) -> u32 {
        // equivalent result, but deliberately expensive (kept from being optimized away)
        let mut s = 0u64;
        for i in 0..2_000_000u64 {
            s = s.wrapping_add(i);
        }
        std::hint::black_box(s);
        *x + 1
    }
}

#[test]
fn autotuner_picks_the_fastest_equivalent_variant() {
    // "slow" is variant 0 (the default) so a successful tune must switch away from it.
    let strat = Strategy::builder()
        .node("slow", Slow)
        .node("fast", Fast)
        .build();
    let handle = strat.handle();

    let mut reg = Registry::new();
    register_primitives(&mut reg);
    let mut g = Graph::new(reg);
    let c = g.add(Const { v: 5 });
    let s = g.add(strat);
    g.connect(c, "out", s, "x").unwrap();

    let mut engine = Mira::compile(&g).unwrap();

    let results = engine.tune(
        &g,
        &[(s, handle.clone())],
        TuneConfig {
            warmup: 1,
            trials: 3,
        },
    );

    assert_eq!(results.len(), 1);
    assert_eq!(
        results[0].chosen_name, "fast",
        "tuner must pick the cheap variant"
    );
    assert_eq!(handle.selected(), results[0].chosen, "tune sets the handle");
    assert!(
        results[0].per_variant_best[0] > results[0].per_variant_best[1],
        "slow variant measured slower than fast"
    );

    // and the variants really were equivalent: result is 6 either way
    let out = *engine
        .run_tick(&g)
        .output(s, "out")
        .unwrap()
        .downcast_ref::<u32>()
        .unwrap();
    assert_eq!(out, 6);
}
