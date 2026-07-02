//! `#[derive(NodeParams)]` — property schemas derived from struct fields.

use octans_core::{Node, ParamKind};
use octans_nodes::*;

#[test]
fn derived_schema_reflects_fields_docs_and_ranges() {
    let s = ThresholdCentroid::node_params();
    let names: Vec<&str> = s.fields.iter().map(|f| f.name).collect();
    assert_eq!(names, ["w", "h", "f", "thr"], "fields in declaration order");

    let thr = &s.fields[3];
    assert_eq!(
        thr.kind,
        ParamKind::Int {
            min: Some(0.0),
            max: Some(255.0)
        },
        "#[param(min, max)] captured"
    );
    assert!(!thr.doc.is_empty(), "doc comment captured");

    let f = &s.fields[2];
    assert_eq!(
        f.kind,
        ParamKind::Float {
            min: Some(1.0),
            max: Some(5000.0)
        }
    );

    // [f64; 3] fields → per-component float widgets.
    let mp = MovingPoint::node_params();
    assert_eq!(mp.fields[0].kind, ParamKind::FloatArray { len: 3 });

    // Unrecognized types fall back to the JSON editor.
    let cam = SyntheticCamera::node_params();
    assert_eq!(cam.fields[2].name, "blobs");
    assert_eq!(cam.fields[2].kind, ParamKind::Json);
}

#[test]
fn node_trait_exposes_the_schema() {
    let node: &dyn Node = &CameraSim {
        center: [0.0; 3],
        w: 64,
        h: 64,
        f: 100.0,
    };
    let schema = node.param_schema().expect("#[node(params)] wires it up");
    assert_eq!(schema.fields.len(), 4);

    // A node without the flag has none (falls back to the JSON editor).
    let plain: &dyn Node = &Triangulate;
    assert!(plain.param_schema().is_none());
}
