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
