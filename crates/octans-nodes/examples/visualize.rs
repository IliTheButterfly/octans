//! Build a small pipeline and print its Graphviz DOT.
//! Run: `cargo run -p octans-nodes --example visualize | dot -Tsvg > graph.svg`

use octans_core::*;
use octans_nodes::*;

fn main() {
    let mut reg = Registry::new();
    register_primitives(&mut reg);
    register_node_types(&mut reg);

    let mut g = Graph::new(reg);
    let cam = g.add(SyntheticCamera {
        w: 128,
        h: 128,
        blobs: vec![(30, 30, 8), (90, 40, 10)],
    });
    let thr = g.add(Threshold);
    let blob = g.add(BlobCount);
    let rep = g.add(Report { label: "cam0" });
    g.connect(cam, "frame", thr, "image").unwrap();
    g.connect(thr, "mask", blob, "mask").unwrap();
    g.connect(blob, "count", rep, "count").unwrap();

    print!("{}", g.to_dot());
}
