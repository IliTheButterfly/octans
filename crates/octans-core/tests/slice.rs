//! The first vertical slice: `SyntheticCamera → Threshold → BlobCount → Report`.
//!
//! Proves the engine spine end-to-end with zero external dependencies:
//! typed / type-erased values, the open type registry, connect-time type checking,
//! topological interpretation by `Mira`, and per-tick latency.

use octans_core::std_nodes::*;
use octans_core::*;

#[test]
fn slice_runs_and_counts_blobs() {
    let mut reg = Registry::new();
    register_std_types(&mut reg);

    let mut g = Graph::new(reg);
    let cam = g.add(SyntheticCamera {
        w: 128,
        h: 128,
        blobs: vec![(30, 30, 8), (90, 40, 10), (60, 100, 6)],
    });
    let thr = g.add(Threshold { thr: 128 });
    let blob = g.add(BlobCount);
    let rep = g.add(Report { label: "cam0" });

    g.connect(cam, "frame", thr, "image").unwrap();
    g.connect(thr, "mask", blob, "mask").unwrap();
    g.connect(blob, "count", rep, "count").unwrap();

    let engine = Mira::compile(&g).expect("graph is acyclic");
    let tick = engine.run_tick(&g);

    let count = tick
        .output(blob, "count")
        .and_then(|v| v.downcast_ref::<u32>())
        .copied()
        .expect("blob count present");

    eprintln!("tick latency: {:?}", tick.latency);
    assert_eq!(count, 3, "should detect exactly the 3 synthetic blobs");
}

#[test]
fn type_mismatch_is_rejected_at_connect_time() {
    let mut reg = Registry::new();
    register_std_types(&mut reg);

    let mut g = Graph::new(reg);
    let cam = g.add(SyntheticCamera { w: 16, h: 16, blobs: vec![] });
    let rep = g.add(Report { label: "x" });

    // frame:Image  ->  count:u32   must be refused before anything runs.
    let result = g.connect(cam, "frame", rep, "count");
    assert!(
        matches!(result, Err(ConnectError::TypeMismatch { .. })),
        "expected a TypeMismatch, got {result:?}"
    );
}
