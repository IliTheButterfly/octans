//! Robustness: a misbehaving node must not take down the engine.

use octans_core::*;
use std::any::Any;

/// A source that always panics in `process`.
struct Bomb;
impl Node for Bomb {
    fn node_type(&self) -> &'static str {
        "test.bomb"
    }
    fn inputs(&self) -> Vec<PortSpec> {
        vec![]
    }
    fn outputs(&self) -> Vec<PortSpec> {
        vec![PortSpec::new("out", i32::type_spec())]
    }
    fn process(&self, _c: &Context, _l: &mut dyn Any, _i: &Inputs, _o: &mut Outputs) {
        panic!("boom");
    }
}

/// A source that produces a value just fine.
struct Steady;
impl Node for Steady {
    fn node_type(&self) -> &'static str {
        "test.steady"
    }
    fn inputs(&self) -> Vec<PortSpec> {
        vec![]
    }
    fn outputs(&self) -> Vec<PortSpec> {
        vec![PortSpec::new("out", i32::type_spec())]
    }
    fn process(&self, _c: &Context, _l: &mut dyn Any, _i: &Inputs, o: &mut Outputs) {
        o.set("out", 7i32);
    }
}

#[test]
fn a_panicking_node_is_isolated_and_the_tick_still_completes() {
    // Silence the default panic hook so the (expected) panic doesn't spam test output.
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));

    let mut reg = Registry::new();
    register_primitives(&mut reg);
    let mut g = Graph::new(reg);
    let bomb = g.add(Bomb);
    let steady = g.add(Steady);

    let mut engine = Mira::compile(&g).unwrap();
    let tick = engine.run_tick(&g);

    std::panic::set_hook(prev);

    // The bomb faulted...
    assert!(!tick.ok());
    assert_eq!(tick.faults.len(), 1);
    assert_eq!(tick.faults[0].node, bomb);
    assert!(tick.faults[0].message.contains("boom"));
    // ...but the engine kept running and the healthy node still produced.
    assert_eq!(
        tick.output(steady, "out").unwrap().downcast_ref::<i32>(),
        Some(&7)
    );

    // And the engine is still usable on the next tick.
    let tick2 = engine.run_tick(&g);
    assert_eq!(tick2.faults.len(), 1);
}
