//! The only window-opening entry point. `cargo test`/`clippy` compile this but never run it, so
//! no display is needed in CI.

use octans_gui::{scene::SceneKind, OctansApp};

fn main() -> eframe::Result<()> {
    eframe::run_native(
        "Octans — watch it run",
        eframe::NativeOptions::default(),
        Box::new(|cc| Ok(Box::new(OctansApp::new(cc, SceneKind::Tracker)))),
    )
}
