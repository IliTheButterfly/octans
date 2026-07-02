//! The Blender-style math/logic node library.

use octans_core::*;
use octans_nodes::*;

/// Evaluate one `Math` op with inputs wired from constants.
fn eval_math(op: &str, a: f64, b: f64, c: f64) -> f64 {
    let mut reg = Registry::new();
    register_primitives(&mut reg);
    let mut g = Graph::new(reg);
    let (va, vb, vc) = (
        g.add(FloatValue { value: a }),
        g.add(FloatValue { value: b }),
        g.add(FloatValue { value: c }),
    );
    let m = g.add(Math { op: op.into() });
    g.connect(va, "value", m, "a").unwrap();
    g.connect(vb, "value", m, "b").unwrap();
    g.connect(vc, "value", m, "c").unwrap();
    let mut e = Mira::compile(&g).unwrap();
    *e.run_tick(&g)
        .output(m, "value")
        .unwrap()
        .downcast_ref::<f64>()
        .unwrap()
}

#[test]
fn math_ops() {
    assert_eq!(eval_math("add", 3.0, 4.0, 0.0), 7.0);
    assert_eq!(eval_math("subtract", 3.0, 4.0, 0.0), -1.0);
    assert_eq!(eval_math("multiply", 3.0, 4.0, 0.0), 12.0);
    assert_eq!(eval_math("divide", 3.0, 4.0, 0.0), 0.75);
    assert_eq!(eval_math("divide", 3.0, 0.0, 0.0), 0.0, "div by zero → 0");
    assert_eq!(eval_math("power", 2.0, 10.0, 0.0), 1024.0);
    assert_eq!(eval_math("sqrt", 81.0, 0.0, 0.0), 9.0);
    assert_eq!(eval_math("sqrt", -1.0, 0.0, 0.0), 0.0, "sqrt(neg) → 0");
    assert_eq!(eval_math("min", 3.0, 4.0, 0.0), 3.0);
    assert_eq!(eval_math("less_than", 3.0, 4.0, 0.0), 1.0);
    assert_eq!(eval_math("compare", 1.0, 1.05, 0.1), 1.0);
    assert_eq!(
        eval_math("fract", -1.25, 0.0, 0.0),
        0.75,
        "fract is positive"
    );
    assert_eq!(eval_math("modulo", 7.0, 3.0, 0.0), 1.0);
    assert_eq!(eval_math("snap", 7.3, 0.5, 0.0), 7.0);
    assert_eq!(
        eval_math("wrap", 5.0, 3.0, 0.0),
        2.0,
        "5 wrapped into [0,3)"
    );
    assert_eq!(eval_math("ping_pong", 5.0, 3.0, 0.0), 1.0);
    assert!((eval_math("atan2", 1.0, 1.0, 0.0) - std::f64::consts::FRAC_PI_4).abs() < 1e-12);
    assert_eq!(eval_math("multiply_add", 2.0, 3.0, 4.0), 10.0);
    assert_eq!(eval_math("degrees", std::f64::consts::PI, 0.0, 0.0), 180.0);
    assert_eq!(eval_math("nonsense", 1.0, 2.0, 3.0), 0.0, "unknown op → 0");
}

#[test]
fn math_defaults_when_unwired() {
    // a/b/c are parameter ports: unconnected → defaults (0.0), so `add` alone yields 0.
    let mut reg = Registry::new();
    register_primitives(&mut reg);
    let mut g = Graph::new(reg);
    let m = g.add(Math { op: "add".into() });
    let mut e = Mira::compile(&g).unwrap();
    let out = *e
        .run_tick(&g)
        .output(m, "value")
        .unwrap()
        .downcast_ref::<f64>()
        .unwrap();
    assert_eq!(out, 0.0);
}

#[test]
fn boolean_compare_and_conversion_chain() {
    // Time → Compare(> 2.5) → BoolToInt: false,false,true… as ticks pass 2.5.
    let mut reg = Registry::new();
    register_primitives(&mut reg);
    let mut g = Graph::new(reg);
    let t = g.add(Time);
    let thr = g.add(FloatValue { value: 2.5 });
    let cmp = g.add(Compare {
        op: "greater".into(),
        epsilon: 1e-9,
    });
    let conv = g.add(BoolToInt);
    g.connect(t, "value", cmp, "a").unwrap();
    g.connect(thr, "value", cmp, "b").unwrap();
    g.connect(cmp, "value", conv, "value").unwrap();

    let mut e = Mira::compile(&g).unwrap();
    let mut outs = Vec::new();
    for _ in 0..4 {
        outs.push(
            *e.run_tick(&g)
                .output(conv, "value")
                .unwrap()
                .downcast_ref::<u32>()
                .unwrap(),
        );
    }
    assert_eq!(outs, [0, 0, 1, 1], "ticks 1,2 ≤ 2.5 < ticks 3,4");
}

#[test]
fn bool_math_truth_table() {
    let eval = |op: &str, a: bool, b: bool| -> bool {
        let mut reg = Registry::new();
        register_primitives(&mut reg);
        let mut g = Graph::new(reg);
        let (va, vb) = (g.add(BoolValue { value: a }), g.add(BoolValue { value: b }));
        let m = g.add(BoolMath { op: op.into() });
        g.connect(va, "value", m, "a").unwrap();
        g.connect(vb, "value", m, "b").unwrap();
        let mut e = Mira::compile(&g).unwrap();
        *e.run_tick(&g)
            .output(m, "value")
            .unwrap()
            .downcast_ref::<bool>()
            .unwrap()
    };
    assert!(eval("and", true, true) && !eval("and", true, false));
    assert!(eval("or", false, true) && !eval("or", false, false));
    assert!(eval("xor", true, false) && !eval("xor", true, true));
    assert!(eval("nand", true, false));
    assert!(!eval("not", true, false));
}

#[test]
fn range_utilities_and_noise() {
    let mut reg = Registry::new();
    register_primitives(&mut reg);
    let mut g = Graph::new(reg);
    let v = g.add(FloatValue { value: 5.0 });
    let mr = g.add(MapRange { clamp: true });
    // remap 5.0 from [0,10] → [0,1] = 0.5
    let (fmin, fmax) = (
        g.add(FloatValue { value: 0.0 }),
        g.add(FloatValue { value: 10.0 }),
    );
    g.connect(v, "value", mr, "value").unwrap();
    g.connect(fmin, "value", mr, "from_min").unwrap();
    g.connect(fmax, "value", mr, "from_max").unwrap();

    let noise = g.add(WhiteNoise { seed: 7 });
    let mut e = Mira::compile(&g).unwrap();
    let t1 = e.run_tick(&g);
    assert_eq!(
        t1.output(mr, "value").unwrap().downcast_ref::<f64>(),
        Some(&0.5)
    );
    let n1 = *t1
        .output(noise, "value")
        .unwrap()
        .downcast_ref::<f64>()
        .unwrap();
    let n2 = *e
        .run_tick(&g)
        .output(noise, "value")
        .unwrap()
        .downcast_ref::<f64>()
        .unwrap();
    assert!((0.0..1.0).contains(&n1) && (0.0..1.0).contains(&n2));
    assert_ne!(n1, n2, "noise re-rolls each tick");
}

#[test]
fn math_nodes_have_op_dropdown_schema_and_round_trip() {
    // The op field is an Enum param (dropdown) with the full option list.
    let schema = Math { op: "add".into() }.param_schema_via_trait();
    let ParamKind::Enum { options } = &schema.fields[0].kind else {
        panic!("op should be an enum param");
    };
    assert!(options.contains(&"multiply_add") && options.len() > 30);

    // And the node round-trips through the serde factory (GraphSpec-save/load-able).
    let mut reg = NodeRegistry::new();
    register_std_factories(&mut reg);
    let cfg = serde_json::json!({"op": "atan2"});
    let n = reg.build("octans.math.math", &cfg).unwrap();
    assert_eq!(n.to_json(), cfg);
}

trait SchemaViaTrait {
    fn param_schema_via_trait(&self) -> ParamSchema;
}
impl<T: Node> SchemaViaTrait for T {
    fn param_schema_via_trait(&self) -> ParamSchema {
        self.param_schema().expect("schema present")
    }
}
