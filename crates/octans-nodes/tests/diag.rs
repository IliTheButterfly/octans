//! Diagnostics nodes: Log emits severity-tagged messages; Probe taps an edge transparently.

use octans_core::*;
use octans_nodes::*;

#[test]
fn log_emits_a_severity_tagged_diagnostic_each_tick() {
    let mut reg = Registry::new();
    register_primitives(&mut reg);
    let mut g = Graph::new(reg);

    // A constant i32 source -> Log at Warning.
    let count = g.add(Const42);
    let logger = g.add(Log::<i32>::warning("blob-count"));
    g.connect(count, "out", logger, "value").unwrap();

    let mut engine = Mira::compile(&g).unwrap();
    let tick = engine.run_tick(&g);

    assert_eq!(tick.diagnostics.len(), 1);
    let d = &tick.diagnostics[0];
    assert_eq!(d.level, LogLevel::Warning);
    assert_eq!(d.source, "blob-count");
    assert_eq!(d.message, "42");
    assert_eq!(d.tick, 1);
}

#[test]
fn probe_passes_through_unchanged_and_records_the_value() {
    let mut reg = Registry::new();
    register_primitives(&mut reg);
    let mut g = Graph::new(reg);

    let count = g.add(Const42);
    let probe = g.add(Probe::<i32>::new("mid"));
    let sink = g.add(Identity);
    g.connect(count, "out", probe, "in").unwrap();
    g.connect(probe, "out", sink, "in").unwrap();

    let mut engine = Mira::compile(&g).unwrap();
    let tick = engine.run_tick(&g);

    // The probe recorded the value...
    assert_eq!(tick.diagnostics.len(), 1);
    assert_eq!(tick.diagnostics[0].source, "mid");
    assert_eq!(tick.diagnostics[0].message, "42");
    // ...and passed it through unchanged: the downstream sink saw 42.
    assert_eq!(
        tick.output(sink, "out").unwrap().downcast_ref::<i32>(),
        Some(&42)
    );
    // The probe's own output equals its input (transparent tap).
    assert_eq!(
        tick.output(probe, "out").unwrap().downcast_ref::<i32>(),
        Some(&42)
    );
}

#[test]
fn log_skips_quietly_when_its_input_is_absent() {
    let mut reg = Registry::new();
    register_primitives(&mut reg);
    let mut g = Graph::new(reg);

    // Source emits only on odd ticks; Log should produce a diagnostic only when fed.
    let src = g.add(OddOnly);
    let logger = g.add(Log::<i32>::info("evt"));
    g.connect(src, "out", logger, "value").unwrap();

    let mut engine = Mira::compile(&g).unwrap();
    let t1 = engine.run_tick(&g); // tick 1, odd -> emits -> log fires
    let t2 = engine.run_tick(&g); // tick 2, even -> no value -> log skipped

    assert_eq!(t1.diagnostics.len(), 1);
    assert!(t2.diagnostics.is_empty());
    assert!(t2.skipped.contains(&logger));
}

#[test]
fn log_fmt_fills_placeholders_from_typed_args() {
    let mut reg = Registry::new();
    register_primitives(&mut reg);
    register_tracking_types(&mut reg);
    let mut g = Graph::new(reg);

    let count = g.add(Const42);
    let point = g.add(ConstPoint);
    let logger = g.add(
        LogFmt::warning("vision", "found {{n}} blobs near {{p}}")
            .arg::<i32>("n")
            .arg::<Pt3>("p"),
    );
    g.connect(count, "out", logger, "n").unwrap();
    g.connect(point, "out", logger, "p").unwrap();

    let mut engine = Mira::compile(&g).unwrap();
    let tick = engine.run_tick(&g);

    assert_eq!(tick.diagnostics.len(), 1);
    let d = &tick.diagnostics[0];
    assert_eq!(d.level, LogLevel::Warning);
    assert_eq!(d.source, "vision");
    assert_eq!(d.message, "found 42 blobs near Pt3([1.0, 2.0, 3.0])");
}

#[test]
fn log_fmt_skips_when_an_arg_is_absent() {
    let mut reg = Registry::new();
    register_primitives(&mut reg);
    let mut g = Graph::new(reg);

    let odd = g.add(OddOnly);
    let always = g.add(Const42);
    let logger = g.add(
        LogFmt::info("evt", "{{a}} / {{b}}")
            .arg::<i32>("a")
            .arg::<i32>("b"),
    );
    g.connect(odd, "out", logger, "a").unwrap();
    g.connect(always, "out", logger, "b").unwrap();

    let mut engine = Mira::compile(&g).unwrap();
    let t1 = engine.run_tick(&g); // odd present -> logs
    let t2 = engine.run_tick(&g); // odd absent -> skipped, no half-filled template

    assert_eq!(t1.diagnostics.len(), 1);
    assert_eq!(t1.diagnostics[0].message, "1 / 42");
    assert!(t2.diagnostics.is_empty());
    assert!(t2.skipped.contains(&logger));
}

// --- tiny test-only source/sink nodes ---

use octans_core::{Inputs, Node, Outputs, PortSpec};
use std::any::Any;

struct ConstPoint;
impl Node for ConstPoint {
    fn node_type(&self) -> &'static str {
        "test.constpoint"
    }
    fn inputs(&self) -> Vec<PortSpec> {
        vec![]
    }
    fn outputs(&self) -> Vec<PortSpec> {
        vec![PortSpec::new("out", Pt3::type_spec())]
    }
    fn process(&self, _c: &Context, _l: &mut dyn Any, _i: &Inputs, o: &mut Outputs) {
        o.set("out", Pt3([1.0, 2.0, 3.0]));
    }
}

struct Const42;
impl Node for Const42 {
    fn node_type(&self) -> &'static str {
        "test.const42"
    }
    fn inputs(&self) -> Vec<PortSpec> {
        vec![]
    }
    fn outputs(&self) -> Vec<PortSpec> {
        vec![PortSpec::new("out", i32::type_spec())]
    }
    fn process(&self, _c: &Context, _l: &mut dyn Any, _i: &Inputs, o: &mut Outputs) {
        o.set("out", 42i32);
    }
}

struct Identity;
impl Node for Identity {
    fn node_type(&self) -> &'static str {
        "test.identity"
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

struct OddOnly;
impl Node for OddOnly {
    fn node_type(&self) -> &'static str {
        "test.oddonly"
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
