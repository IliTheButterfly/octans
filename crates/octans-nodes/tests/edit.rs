//! Editor-support graph API: validate a wire without mutating, and disconnect an input.

use octans_core::*;
use octans_nodes::*;

#[test]
fn can_connect_validates_without_mutating_and_disconnect_removes() {
    let mut reg = Registry::new();
    register_primitives(&mut reg);
    register_node_types(&mut reg);
    let mut g = Graph::new(reg);
    let cam = g.add(SyntheticCamera {
        w: 8,
        h: 8,
        blobs: vec![],
    });
    let thr = g.add(Threshold);

    // A valid wire validates; a type-mismatched one is refused — neither mutates the graph.
    assert!(g.can_connect(cam, "frame", thr, "image").is_ok());
    assert!(
        g.can_connect(cam, "frame", thr, "thr").is_err(),
        "Image → u8 is a type mismatch"
    );

    g.connect(cam, "frame", thr, "image").unwrap();
    assert_eq!(g.disconnect_input(thr, "image"), 1, "removes the one edge");
    assert_eq!(g.disconnect_input(thr, "image"), 0, "idempotent");
    // After disconnect the required input is unwired again.
    assert!(matches!(
        Mira::compile(&g),
        Err(CompileError::UnconnectedInput { .. })
    ));
}

#[test]
fn config_nodes_rebuild_from_edited_json() {
    let mut factories = NodeRegistry::new();
    register_std_factories(&mut factories);

    // A param edit is: take config JSON, tweak it, rebuild the node from it.
    let cfg = serde_json::json!({"center": [1.0, 2.0, 3.0], "w": 32, "h": 16, "f": 250.0});
    let node = factories
        .build("octans.track.camera_sim", &cfg)
        .expect("camera_sim has a serde factory now");
    assert_eq!(node.node_type(), "octans.track.camera_sim");
    // The rebuilt node's config round-trips to what we asked for (int/float flavors preserved).
    assert_eq!(node.to_json(), cfg);
}

#[test]
fn remove_node_tombstones_and_keeps_other_ids_stable() {
    let mut reg = Registry::new();
    register_primitives(&mut reg);
    register_node_types(&mut reg);
    let mut g = Graph::new(reg);
    let cam = g.add(SyntheticCamera {
        w: 8,
        h: 8,
        blobs: vec![],
    });
    let thr = g.add(Threshold);
    g.connect(cam, "frame", thr, "image").unwrap();
    let blob = g.add(BlobCount);
    g.connect(thr, "mask", blob, "mask").unwrap();

    // Remove the middle node: its edges drop, its slot becomes a tombstone, and the *other*
    // NodeIds keep pointing at the same nodes (no renumbering).
    g.remove_node(thr);
    assert_eq!(
        g.node(thr).unwrap().node_type(),
        octans_core::TOMBSTONE_TYPE
    );
    assert_eq!(
        g.node(cam).unwrap().node_type(),
        "octans.std.synthetic_camera"
    );
    assert_eq!(g.node(blob).unwrap().node_type(), "octans.std.blob_count");
    assert_eq!(g.node_count(), 3, "the slot is retained, not removed");

    // blob's `mask` lost its feeder → won't compile until rewired; rewiring fixes it.
    assert!(matches!(
        Mira::compile(&g),
        Err(CompileError::UnconnectedInput { .. })
    ));
    g.connect(cam, "frame", blob, "mask").unwrap();
    assert!(Mira::compile(&g).is_ok());
}
