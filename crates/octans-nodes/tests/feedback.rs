//! The first self-correcting loop — the thing that makes this Octans and not just a node graph.
//!
//! Pipeline: `Camera → Threshold → BlobCount`. An `AutoThreshold` optimizer observes the blob
//! count from the *previous* tick (via a portal) and drives `Threshold.thr` this tick, with its
//! own previous output fed back through a second portal (pure node, explicit state). Starting
//! from a deliberately bad threshold (255 → zero blobs), the loop must steer the threshold into
//! the range that recovers the 3 real blobs.

use octans_core::*;
use octans_nodes::*;

#[test]
fn optimizer_self_corrects_threshold_to_hit_target() {
    let mut reg = Registry::new();
    register_primitives(&mut reg);
    register_node_types(&mut reg);
    let mut g = Graph::new(reg);

    // Feedback state, carried explicitly across ticks:
    let p_count = g.add_portal(<u32 as RegisteredType>::type_spec(), Value::new(0u32));
    let p_thr = g.add_portal(<u8 as RegisteredType>::type_spec(), Value::new(255u8)); // bad start

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
    let thr_r = g.add(p_thr.reader("thr"));
    let thr_w = g.add(p_thr.writer("thr"));

    // optimizer observes last tick's count + its own last output
    g.connect(count_r, "count", opt, "count").unwrap();
    g.connect(thr_r, "thr", opt, "prev_thr").unwrap();
    // optimizer drives the threshold this tick + records it for next tick
    g.connect(opt, "thr", thr_node, "thr").unwrap();
    g.connect(opt, "thr", thr_w, "thr").unwrap();
    // forward pipeline
    g.connect(cam, "frame", thr_node, "image").unwrap();
    g.connect(thr_node, "mask", blob, "mask").unwrap();
    g.connect(blob, "count", count_w, "count").unwrap();

    let engine = Mira::compile(&g).expect("acyclic via the two portals");

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
