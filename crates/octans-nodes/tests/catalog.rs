//! The node catalog lists the standard node types with their derived metadata + ports.

use octans_core::Catalog;
use octans_nodes::register_std_catalog;

#[test]
fn catalog_lists_std_nodes_with_ports() {
    let mut cat = Catalog::new();
    register_std_catalog(&mut cat);
    assert!(
        cat.len() >= 12,
        "expected a populated catalog, got {}",
        cat.len()
    );

    let thr = cat
        .get("octans.std.threshold")
        .expect("threshold is in the catalog");
    assert_eq!(thr.display_name, "threshold");
    assert_eq!(thr.category, "std");
    assert!(thr.inputs.iter().any(|(n, _, _)| n == "image"));
    assert!(thr.outputs.iter().any(|(n, _)| n == "mask"));

    // tracking + core categories present, grouped
    let by = cat.by_category();
    assert!(by.contains_key("std"));
    assert!(by.contains_key("track"));
    assert!(by.contains_key("core"), "Gather/Scatter land under core");
}

#[test]
fn catalog_constructs_fresh_nodes() {
    let mut cat = Catalog::new();
    register_std_catalog(&mut cat);

    let node = cat
        .make("octans.std.threshold")
        .expect("can build a threshold");
    assert_eq!(node.node_type(), "octans.std.threshold");
    // each call yields an independent instance
    let a = cat.make("octans.std.blob_count").unwrap();
    let b = cat.make("octans.std.blob_count").unwrap();
    assert_eq!(a.node_type(), b.node_type());

    assert!(cat.make("octans.std.does_not_exist").is_none());
}
