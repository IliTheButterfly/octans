//! Runtime-typed construction of the generic structural nodes — the engine foundation for
//! instantiating them from an editor palette (element type chosen from a dropdown, not a
//! turbofish).

use octans_core::*;
use octans_nodes::*;

#[test]
fn new_dyn_matches_the_typed_constructors_and_runs() {
    // Same ports as the turbofish version.
    let a = Gather::new::<Px>(3);
    let b = Gather::new_dyn(Px::type_spec(), 3);
    let names = |n: &dyn Node| n.inputs().iter().map(|p| p.name).collect::<Vec<_>>();
    assert_eq!(names(&a), names(&b));
    assert_eq!(a.outputs()[0].ty, b.outputs()[0].ty);

    // And a graph built purely from runtime TypeSpecs works end to end.
    let mut reg = Registry::new();
    register_primitives(&mut reg);
    let f64_spec = TypeSpec::scalar("octans.f64");
    let mut g = Graph::new(reg);
    let src = g.add(VecConst {
        items: vec![1.0f64, 2.0, 3.0],
    });
    let scatter = g.add(Scatter::new_dyn(f64_spec.clone(), 3));
    let gather = g.add(Gather::new_dyn(f64_spec, 3));
    g.connect(src, "items", scatter, "items").unwrap();
    for i in 0..3 {
        g.connect(scatter, &format!("out{i}"), gather, &format!("in{i}"))
            .unwrap();
    }
    let mut engine = Mira::compile(&g).unwrap();
    let t = engine.run_tick(&g);
    let items = t.output(gather, "items").unwrap().as_vector().unwrap();
    assert_eq!(items.len(), 3);
    assert_eq!(items[1].downcast_ref::<f64>(), Some(&2.0));
}

#[test]
fn registry_enumerates_types_for_a_dropdown() {
    let mut reg = Registry::new();
    register_primitives(&mut reg);
    register_node_types(&mut reg);
    let ids: Vec<&str> = reg.iter_types().map(|d| d.id).collect();
    assert!(ids.contains(&"octans.f64"));
    assert!(ids.contains(&"octans.std.image"));
    assert!(ids.len() >= 10);
}
