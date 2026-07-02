//! The node palette — search, browse by category, ➕ to add. Includes a **structural** section
//! where Gather/Scatter/Switch are instantiated with a chosen element type and arity (via the
//! engine's `new_dyn` constructors), so parallel fan-in/out is buildable from the UI.

use crate::model::type_label;
use crate::OctansApp;
use eframe::egui::{self, Color32};

impl OctansApp {
    pub(crate) fn palette_window(&mut self, ctx: &egui::Context) {
        if !self.show_palette {
            return;
        }
        let mut open = true;
        let mut to_add: Option<&'static str> = None;
        let mut structural: Option<&'static str> = None;

        egui::Window::new("node palette")
            .open(&mut open)
            .default_width(320.0)
            .resizable(true)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("🔍");
                    ui.text_edit_singleline(&mut self.palette_filter);
                    if ui.small_button("✕").clicked() {
                        self.palette_filter.clear();
                    }
                });
                ui.separator();

                // --- structural: element-typed fan-in/out with chosen arity ---
                ui.strong("structural");
                ui.horizontal(|ui| {
                    let mut type_ids: Vec<(&'static str, &'static str)> = self
                        .graph
                        .registry()
                        .iter_types()
                        .map(|d| (d.id, d.name))
                        .collect();
                    type_ids.sort_by_key(|(id, _)| *id);
                    egui::ComboBox::from_id_salt("palette-elem")
                        .selected_text(
                            self.palette_elem
                                .rsplit('.')
                                .next()
                                .unwrap_or(&self.palette_elem)
                                .to_string(),
                        )
                        .show_ui(ui, |ui| {
                            for (id, name) in &type_ids {
                                if ui
                                    .selectable_label(self.palette_elem == *id, *name)
                                    .clicked()
                                {
                                    self.palette_elem = (*id).to_string();
                                }
                            }
                        });
                    ui.label("×");
                    ui.add(egui::DragValue::new(&mut self.palette_arity).range(1..=16));
                });
                ui.horizontal(|ui| {
                    if ui
                        .button("➕ gather")
                        .on_hover_text("N scalars → vector")
                        .clicked()
                    {
                        structural = Some("octans.core.gather");
                    }
                    if ui
                        .button("➕ scatter")
                        .on_hover_text("vector → N scalars")
                        .clicked()
                    {
                        structural = Some("octans.core.scatter");
                    }
                    if ui
                        .button("➕ switch")
                        .on_hover_text("route one of N inputs (select: u32)")
                        .clicked()
                    {
                        structural = Some("octans.core.switch");
                    }
                });
                ui.separator();

                // --- group templates: capture / instantiate / map / loop ---
                self.templates_section(ui);
                ui.separator();

                // --- catalog, filtered + grouped ---
                let filter = self.palette_filter.to_lowercase();
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for (category, classes) in self.catalog.by_category() {
                            let visible: Vec<_> = classes
                                .iter()
                                .filter(|c| {
                                    filter.is_empty()
                                        || c.display_name.contains(&filter)
                                        || c.type_id.contains(&filter)
                                })
                                .collect();
                            if visible.is_empty() {
                                continue;
                            }
                            egui::CollapsingHeader::new(category)
                                .default_open(!filter.is_empty() || category != "std")
                                .show(ui, |ui| {
                                    for c in visible {
                                        ui.horizontal(|ui| {
                                            if ui
                                                .small_button("➕")
                                                .on_hover_text("add to graph")
                                                .clicked()
                                            {
                                                to_add = Some(c.type_id);
                                            }
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

        // Apply adds after the window closure (avoids borrowing self.graph while iterating
        // self.catalog). Append-only — safe w.r.t. NodeId indices — recorded + recompiled.
        if let Some(type_id) = to_add {
            self.add_node_from_catalog(type_id);
        }
        if let Some(type_id) = structural {
            let cfg = serde_json::json!({
                "elem": self.palette_elem,
                "arity": self.palette_arity,
            });
            self.add_node_with_config(type_id, cfg);
        }
    }
}
