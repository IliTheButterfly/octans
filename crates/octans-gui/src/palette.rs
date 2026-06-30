//! The node palette — a browsable list of available node types, grouped by category, with each
//! type's port signature. Read-only for now (drag-to-add arrives with the editor); it's the
//! visible front of the node [`Catalog`](octans_core::Catalog).

use crate::model::type_label;
use crate::OctansApp;
use eframe::egui::{self, Color32};

impl OctansApp {
    pub(crate) fn palette_window(&mut self, ctx: &egui::Context) {
        if !self.show_palette {
            return;
        }
        let mut open = true;
        egui::Window::new("node palette")
            .open(&mut open)
            .default_width(300.0)
            .resizable(true)
            .show(ctx, |ui| {
                ui.weak(format!(
                    "{} node types — browse (drag-to-add lands with the editor)",
                    self.catalog.len()
                ));
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for (category, classes) in self.catalog.by_category() {
                            egui::CollapsingHeader::new(category)
                                .default_open(true)
                                .show(ui, |ui| {
                                    for c in classes {
                                        ui.horizontal(|ui| {
                                            ui.strong(&c.display_name);
                                            ui.weak(c.type_id);
                                        });
                                        for (name, ty, opt) in &c.inputs {
                                            let o = if *opt { " ?" } else { "" };
                                            ui.colored_label(
                                                Color32::from_gray(150),
                                                format!("    ‹ {name}: {}{o}", type_label(ty)),
                                            );
                                        }
                                        for (name, ty) in &c.outputs {
                                            ui.colored_label(
                                                Color32::from_rgb(150, 190, 150),
                                                format!("    › {name}: {}", type_label(ty)),
                                            );
                                        }
                                        ui.add_space(4.0);
                                    }
                                });
                        }
                    });
            });
        if !open {
            self.show_palette = false;
        }
    }
}
