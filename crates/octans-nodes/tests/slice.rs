//! The vertical slice, now authored with `#[node]`: `SyntheticCamera → Threshold → BlobCount → Report`.
//!
//! Same end-to-end proof as the v0 slice, but the nodes are written with the macro and live in
//! a *separate crate* from the engine — validating the real plugin-author path.

use octans_core::*;
use octans_nodes::*;

fn registry() -> Registry {
    let mut reg = Registry::new();
    register_primitives(&mut reg); // u32, f32, ... (from octans-core)
    register_node_types(&mut reg); // Image (from octans-nodes)
    reg
}

#[test]
fn slice_runs_and_counts_blobs() {
    let mut g = Graph::new(registry());
    let cam = g.add(SyntheticCamera {
        w: 128,
        h: 128,
        blobs: vec![(30, 30, 8), (90, 40, 10), (60, 100, 6)],
    });
    let thr = g.add(Threshold); // thr param unconnected -> default 128
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
    let mut g = Graph::new(registry());
    let cam = g.add(SyntheticCamera {
        w: 16,
        h: 16,
        blobs: vec![],
    });
    let rep = g.add(Report { label: "x" });

    // frame:Image  ->  count:u32   must be refused before anything runs.
    let result = g.connect(cam, "frame", rep, "count");
    assert!(
        matches!(result, Err(ConnectError::TypeMismatch { .. })),
        "expected a TypeMismatch, got {result:?}"
    );
}
