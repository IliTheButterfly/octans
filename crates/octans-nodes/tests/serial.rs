//! A graph serializes to data (JSON) and rebuilds into a runnable graph that behaves identically.

use octans_core::*;
use octans_nodes::*;

fn registry() -> Registry {
    let mut reg = Registry::new();
    register_primitives(&mut reg);
    register_node_types(&mut reg);
    reg
}

#[test]
fn graph_round_trips_through_json() {
    // Build a graph: camera -> threshold -> blob count.
    let mut g = Graph::new(registry());
    let cam = g.add(SyntheticCamera {
        w: 96,
        h: 96,
        blobs: vec![(20, 20, 6), (60, 60, 7), (20, 70, 5)],
    });
    let thr = g.add(Threshold);
    let blob = g.add(BlobCount);
    g.connect(cam, "frame", thr, "image").unwrap();
    g.connect(thr, "mask", blob, "mask").unwrap();

    let original = {
        let mut e = Mira::compile(&g).unwrap();
        *e.run_tick(&g)
            .output(blob, "count")
            .unwrap()
            .downcast_ref::<u32>()
            .unwrap()
    };
    assert_eq!(original, 3);

    // Serialize -> JSON string -> back to a spec.
    let spec = g.to_spec();
    let json = serde_json::to_string_pretty(&spec).unwrap();
    assert!(json.contains("octans.std.synthetic_camera"));
    let spec2: GraphSpec = serde_json::from_str(&json).unwrap();

    // Rebuild a live graph from the spec via the node factories.
    let mut factories = NodeRegistry::new();
    register_std_factories(&mut factories);
    let g2 = spec2.build(registry(), &factories).expect("rebuild");

    let rebuilt = {
        let mut e = Mira::compile(&g2).unwrap();
        *e.run_tick(&g2)
            .output(blob, "count")
            .unwrap()
            .downcast_ref::<u32>()
            .unwrap()
    };
    assert_eq!(rebuilt, original, "the rebuilt graph behaves identically");
}
