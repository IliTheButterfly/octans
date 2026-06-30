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
