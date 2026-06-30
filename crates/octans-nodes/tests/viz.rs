//! A graph renders to Graphviz DOT (the headless visualizer).

use octans_core::*;
use octans_nodes::*;

#[test]
fn graph_renders_to_dot() {
    let mut reg = Registry::new();
    register_primitives(&mut reg);
    register_node_types(&mut reg);
    let mut g = Graph::new(reg);

    let cam = g.add(SyntheticCamera {
        w: 32,
        h: 32,
        blobs: vec![(8, 8, 3)],
    });
    let thr = g.add(Threshold);
    let blob = g.add(BlobCount);
    g.connect(cam, "frame", thr, "image").unwrap();
    g.connect(thr, "mask", blob, "mask").unwrap();

    let dot = g.to_dot();
    assert!(dot.starts_with("digraph octans {"));
    assert!(dot.trim_end().ends_with('}'));
    assert!(dot.contains("octans.std.threshold"));
    assert!(dot.contains("<in_image>") && dot.contains("<out_frame>"));
    assert_eq!(dot.matches("->").count(), 2, "one arrow per edge");
}
