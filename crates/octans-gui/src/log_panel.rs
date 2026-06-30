//! The Log panel: streams `Tick::diagnostics` (accumulated in the `LogRing`), color-coded by level.

use crate::OctansApp;
use eframe::egui::{self, Color32};
use octans_core::LogLevel;

fn level_color(level: LogLevel) -> Color32 {
    match level {
        LogLevel::Error => Color32::from_rgb(240, 90, 90),
        LogLevel::Warning => Color32::from_rgb(230, 180, 60),
        LogLevel::Info => Color32::from_gray(170),
    }
}

impl OctansApp {
    pub(crate) fn log_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.strong("Log");
            if ui.button("Clear").clicked() {
                self.log.clear();
            }
            ui.checkbox(&mut self.follow_log, "follow");
            ui.label(format!("{} lines", self.log.len()));
        });
        ui.separator();

        egui::ScrollArea::vertical()
            .stick_to_bottom(self.follow_log)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for d in self.log.iter() {
                    ui.horizontal(|ui| {
                        ui.monospace(format!("{:>5}", d.tick));
                        ui.colored_label(level_color(d.level), format!("{:<5}", d.level));
                        ui.monospace(format!("{:<14}", d.source));
                        ui.label(&d.message);
                    });
                }
            });
    }
}
