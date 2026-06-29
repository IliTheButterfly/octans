//! The first self-correcting loop — now with the optimizer's controller memory in **local
//! state** and a single observation portal.
//!
//! Pipeline: `Camera → Threshold → BlobCount`. `AutoThreshold` keeps its current threshold as
//! node-local state, observes the previous tick's blob count via one portal, and drives
//! `Threshold.thr`. From a deliberately bad start (local thr=255 → zero blobs) it steers the
//! threshold down into the range that recovers the 3 real blobs.

use octans_core::*;
use octans_nodes::*;

#[test]
fn optimizer_self_corrects_threshold_to_hit_target() {
    let mut reg = Registry::new();
    register_primitives(&mut reg);
    register_node_types(&mut reg);
    let mut g = Graph::new(reg);

    // one portal: last tick's blob count (the threshold itself is now the optimizer's local state)
    let p_count = g.add_portal(<u32 as RegisteredType>::type_spec(), Value::new(0u32));

    let cam = g.add(SyntheticCamera {
        w: 128,
        h: 128,
        blobs: vec![(30, 30, 8), (90, 40, 10), (60, 100, 6)],
    });
    let thr_node = g.add(Threshold);
    let blob = g.add(BlobCount);
    let opt = g.add(AutoThreshold {
        target: 3,
        gain: 5,
        min: 0,
        max: 255,
    });
    let count_r = g.add(p_count.reader("count"));
    let count_w = g.add(p_count.writer("count"));

    g.connect(count_r, "count", opt, "count").unwrap();
    g.connect(opt, "thr", thr_node, "thr").unwrap();
    g.connect(cam, "frame", thr_node, "image").unwrap();
    g.connect(thr_node, "mask", blob, "mask").unwrap();
    g.connect(blob, "count", count_w, "count").unwrap();

    let mut engine = Mira::compile(&g).expect("acyclic via the count portal");

    let mut counts = Vec::new();
    let mut thrs = Vec::new();
    for _ in 0..8 {
        let tick = engine.run_tick(&g);
        counts.push(
            tick.output(blob, "count")
                .and_then(|v| v.downcast_ref::<u32>())
                .copied()
                .unwrap(),
        );
        thrs.push(
            tick.output(opt, "thr")
                .and_then(|v| v.downcast_ref::<u8>())
                .copied()
                .unwrap(),
        );
    }
    eprintln!("blob counts per tick: {counts:?}");
    eprintln!("thresholds per tick:  {thrs:?}");

    assert_ne!(
        counts[0], 3,
        "must start off-target (threshold too high to see any blob)"
    );
    assert_eq!(
        *counts.last().unwrap(),
        3,
        "the optimizer must self-correct the threshold to recover the 3 blobs"
    );
}
