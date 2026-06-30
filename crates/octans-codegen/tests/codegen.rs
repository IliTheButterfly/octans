//! Codegen lowers a graph's IR to valid Rust source with the structure baked in.

use octans_codegen::emit_rust;
use octans_core::*;
use octans_nodes::*;

#[test]
fn emits_valid_rust_for_a_graph() {
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

    let src = emit_rust(&g.to_spec());

    // The generated source must be valid Rust…
    syn::parse_file(&src).expect("generated code parses as Rust");
    // …and bake in the graph's structure.
    assert!(src.contains("octans.std.synthetic_camera"));
    assert!(src.contains("octans.std.threshold"));
    assert_eq!(
        src.matches("g.connect").count(),
        2,
        "one statement per edge"
    );
    assert!(src.contains("pub fn build"));
}
