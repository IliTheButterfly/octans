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

/// A source that emits only on odd ticks, and nothing on even ticks.
struct Intermittent;
impl Node for Intermittent {
    fn node_type(&self) -> &'static str {
        "test.intermittent"
    }
    fn inputs(&self) -> Vec<PortSpec> {
        vec![]
    }
    fn outputs(&self) -> Vec<PortSpec> {
        vec![PortSpec::new("out", i32::type_spec())]
    }
    fn process(&self, c: &Context, _l: &mut dyn Any, _i: &Inputs, o: &mut Outputs) {
        if c.tick() % 2 == 1 {
            o.set("out", 1i32);
        }
    }
}

/// Consumes a required input and echoes it.
struct Echo;
impl Node for Echo {
    fn node_type(&self) -> &'static str {
        "test.echo"
    }
    fn inputs(&self) -> Vec<PortSpec> {
        vec![PortSpec::new("in", i32::type_spec())]
    }
    fn outputs(&self) -> Vec<PortSpec> {
        vec![PortSpec::new("out", i32::type_spec())]
    }
    fn process(&self, _c: &Context, _l: &mut dyn Any, i: &Inputs, o: &mut Outputs) {
        o.set("out", *i.get::<i32>("in"));
    }
}

#[test]
fn a_missing_required_input_skips_the_node_and_cascades() {
    let mut reg = Registry::new();
    register_primitives(&mut reg);
    let mut g = Graph::new(reg);
    let src = g.add(Intermittent);
    let echo = g.add(Echo);
    g.connect(src, "out", echo, "in").unwrap();

    let mut engine = Mira::compile(&g).unwrap();

    // Tick 1 (odd): source emits, echo runs.
    let t1 = engine.run_tick(&g);
    assert!(t1.skipped.is_empty());
    assert_eq!(
        t1.output(echo, "out").unwrap().downcast_ref::<i32>(),
        Some(&1)
    );

    // Tick 2 (even): source emits nothing -> echo's required input is absent -> echo is skipped,
    // not fed garbage and not panicking. No output, recorded in `skipped`.
    let t2 = engine.run_tick(&g);
    assert_eq!(t2.skipped, vec![echo]);
    assert!(t2.output(echo, "out").is_none());
    assert!(t2.ok()); // a skip is not a fault
}

#[test]
fn compile_rejects_an_unconnected_required_input() {
    let mut reg = Registry::new();
    register_primitives(&mut reg);
    let mut g = Graph::new(reg);
    // Echo's required `in` is left unconnected: it could never receive a value.
    let echo = g.add(Echo);

    match Mira::compile(&g) {
        Err(CompileError::UnconnectedInput { node, port }) => {
            assert_eq!(node, echo);
            assert_eq!(port, "in");
        }
        Err(e) => panic!("expected UnconnectedInput, got {e:?}"),
        Ok(_) => panic!("expected compile to fail on unconnected required input"),
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
