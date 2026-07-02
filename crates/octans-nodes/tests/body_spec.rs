//! Groups **as data**: a `BodySpec` lowers to a live `GroupTemplate` (via serde factories) that
//! instantiates, nests under `Map` (data-parallel), and under `Loop` (for-loops) — the whole
//! structural family, authored from data instead of Rust closures. This is the engine keystone
//! for editing groups/loops/parallel in the GUI.

use octans_core::*;
use octans_nodes::*;
use std::sync::Arc;

fn factories() -> Arc<NodeRegistry> {
    let mut f = NodeRegistry::new();
    register_std_factories(&mut f);
    Arc::new(f)
}

/// A data-defined "affine" group: value → (×2) → (+10), boundary in `x`, out `y`.
fn affine_spec() -> BodySpec {
    serde_json::from_value(serde_json::json!({
        "nodes": [
            { "type": "octans.math.math", "config": { "op": "multiply" } },
            { "type": "octans.math.math", "config": { "op": "add" } },
            { "type": "octans.math.value", "config": { "value": 2.0 } },
            { "type": "octans.math.value", "config": { "value": 10.0 } },
        ],
        "edges": [
            { "from": 2, "from_port": "value", "to": 0, "to_port": "b" },
            { "from": 0, "from_port": "value", "to": 1, "to_port": "a" },
            { "from": 3, "from_port": "value", "to": 1, "to_port": "b" },
        ],
        "inputs":  [ { "name": "x", "node": 0, "port": "a" } ],
        "outputs": [ { "name": "y", "node": 1, "port": "value" } ],
    }))
    .unwrap()
}

#[test]
fn body_spec_lowers_and_instantiates_independently() {
    let tpl = GroupTemplate::from_spec("affine", affine_spec(), factories()).unwrap();

    let mut reg = Registry::new();
    register_primitives(&mut reg);
    let mut g = Graph::new(reg);
    let src = g.add(FloatValue { value: 5.0 });
    // two independent instances of the same data template
    let g1 = g.add_group(&tpl).unwrap();
    let g2 = g.add_group(&tpl).unwrap();
    let (n1, p1) = g1.input("x");
    let (n2, p2) = g2.input("x");
    g.connect(src, "value", n1, p1).unwrap();
    g.connect(src, "value", n2, p2).unwrap();

    let mut e = Mira::compile(&g).unwrap();
    let t = e.run_tick(&g);
    let (o1, q1) = g1.output("y");
    let (o2, q2) = g2.output("y");
    assert_eq!(t.output(o1, q1).unwrap().downcast_ref::<f64>(), Some(&20.0));
    assert_eq!(t.output(o2, q2).unwrap().downcast_ref::<f64>(), Some(&20.0));
}

#[test]
fn body_spec_group_runs_under_map_and_loop() {
    let facs = factories();

    // Map over the data-defined group: [1,2,3] → [12, 14, 16] (x*2+10), in parallel lanes.
    let tpl = GroupTemplate::from_spec("affine", affine_spec(), facs.clone()).unwrap();
    let mut reg = Registry::new();
    register_primitives(&mut reg);
    let mut g = Graph::new(reg);
    let src = g.add(VecConst {
        items: vec![1.0f64, 2.0, 3.0],
    });
    let map = g.add(Map::group(&tpl));
    g.connect(src, "items", map, "x").unwrap();

    // Loop over the data-defined group: apply x*2+10 three times to 1 → ((1·2+10)·2+10)·2+10 = 78.
    let tpl2 = GroupTemplate::from_spec("affine", affine_spec(), facs).unwrap();
    let one = g.add(FloatValue { value: 1.0 });
    let lp = g.add(Loop::group(3, &tpl2));
    g.connect(one, "value", lp, "x").unwrap();

    let mut e = Mira::compile(&g).unwrap();
    let t = e.run_tick(&g);

    let mapped = t.output(map, "y").unwrap().as_vector().unwrap();
    let vals: Vec<f64> = mapped
        .iter()
        .map(|v| *v.downcast_ref::<f64>().unwrap())
        .collect();
    assert_eq!(vals, [12.0, 14.0, 16.0]);

    assert_eq!(
        t.output(lp, "y").unwrap().downcast_ref::<f64>(),
        Some(&78.0)
    );
}

#[test]
fn from_spec_rejects_unknown_node_types_up_front() {
    let mut spec = affine_spec();
    spec.nodes[0].type_id = "octans.does.not_exist".into();
    match GroupTemplate::from_spec("bad", spec, factories()) {
        Err(BuildError::UnknownNodeType(t)) => assert_eq!(t, "octans.does.not_exist"),
        other => panic!("expected UnknownNodeType, got {:?}", other.map(|_| ())),
    }
}

#[test]
fn body_spec_serializes() {
    // The whole point: a group definition round-trips as JSON.
    let json = serde_json::to_string(&affine_spec()).unwrap();
    let back: BodySpec = serde_json::from_str(&json).unwrap();
    assert_eq!(back.nodes.len(), 4);
    assert_eq!(back.inputs[0].name, "x");
}
