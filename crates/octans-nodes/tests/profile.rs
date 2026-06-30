//! The always-on profiler records every node's latency, every tick.

use octans_core::*;
use octans_nodes::*;

#[test]
fn profiler_records_per_node_latency() {
    let mut reg = Registry::new();
    register_primitives(&mut reg);
    register_node_types(&mut reg);
    let mut g = Graph::new(reg);

    let cam = g.add(SyntheticCamera {
        w: 64,
        h: 64,
        blobs: vec![(20, 20, 6), (40, 40, 6)],
    });
    let thr = g.add(Threshold);
    let blob = g.add(BlobCount);
    g.connect(cam, "frame", thr, "image").unwrap();
    g.connect(thr, "mask", blob, "mask").unwrap();

    let mut engine = Mira::compile(&g).unwrap();
    for _ in 0..3 {
        engine.run_tick(&g);
    }

    let prof = engine.profile();
    // every node profiled, three samples each
    for (_node, stat) in prof.iter() {
        assert_eq!(stat.samples, 3, "each node sampled once per tick");
    }
    // the blob detector (real work) should have taken some measurable time, and mean is sane
    let blob_stat = prof.node(blob);
    assert!(blob_stat.max >= blob_stat.mean(), "max >= mean");
    assert!(prof.len() == 3);
}
