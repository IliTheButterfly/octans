//! The add-node editing semantics the GUI relies on: appending a catalog node and recompiling.
//! (The `OctansApp` itself needs an egui context, so we exercise the graph-level behavior here.)

use octans_core::{Catalog, CompileError, Mira};
use octans_gui::scene::SceneKind;
use octans_nodes::register_std_catalog;

#[test]
fn add_source_compiles_but_unconnected_required_input_does_not() {
    let mut cat = Catalog::new();
    register_std_catalog(&mut cat);
    let mut scene = SceneKind::Diagnostics.build();

    // Adding a source (a camera has no required inputs) keeps the graph compiling.
    scene
        .graph
        .add_boxed(cat.make("octans.std.synthetic_camera").unwrap());
    assert!(Mira::compile(&scene.graph).is_ok());

    // Adding a node whose required input is now unconnected makes it (intentionally) not compile —
    // the GUI keeps the edit and surfaces this until the port is wired.
    scene
        .graph
        .add_boxed(cat.make("octans.std.threshold").unwrap());
    match Mira::compile(&scene.graph) {
        Err(CompileError::UnconnectedInput { .. }) => {}
        Err(e) => panic!("expected UnconnectedInput, got {e:?}"),
        Ok(_) => panic!("expected a compile error for the unconnected threshold"),
    }
}
