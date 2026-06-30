//! The only window-opening entry point. `cargo test`/`clippy` compile this but never run it, so
//! no display is needed in CI.

use octans_gui::{scene::SceneKind, OctansApp};

fn main() -> eframe::Result<()> {
    // Quiet by default (deps like winit/zbus are noisy); override with e.g. `RUST_LOG=debug`.
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("warn,octans_gui=info"),
    )
    .init();
    log::info!("octans-gui starting");

    eframe::run_native(
        "Octans — watch it run",
        eframe::NativeOptions::default(),
        Box::new(|cc| Ok(Box::new(OctansApp::new(cc, SceneKind::Tracker)))),
    )
}
