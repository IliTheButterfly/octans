//! The Pyxis autotuner panel — trigger a tune, watch per-variant benchmarks, and switch variants
//! live. Only shown when the scene has `Strategy` nodes.
//!
//! The autotuner benchmarks each variant on *this* hardware and keeps the fastest one whose outputs
//! still match the reference (variant 0) — so the panel shows both speed bars and a verified /
//! rejected verdict, not just a winner.

use crate::{fmt_dur, OctansApp};
use eframe::egui::{self, Color32};
use octans_core::TuneConfig;

impl OctansApp {
    pub(crate) fn autotuner_ui(&mut self, ui: &mut egui::Ui) {
        if self.strategies.is_empty() {
            return;
        }
        ui.strong("Pyxis · autotuner");
        ui.horizontal(|ui| {
            ui.label("warmup");
            ui.add(egui::DragValue::new(&mut self.tune_warmup).range(0..=20));
            ui.label("trials");
            ui.add(egui::DragValue::new(&mut self.tune_trials).range(1..=50));
        });

        if ui.button("⏱ Tune").clicked() {
            let cfg = TuneConfig {
                warmup: self.tune_warmup,
                trials: self.tune_trials,
            };
            let results = self.engine.tune(&self.graph, &self.strategies, cfg);
            self.tune_results.clear();
            for r in results {
                self.tune_results.insert(r.node.0, r);
            }
            // Refresh the displayed tick with the chosen variant now active.
            let tick = self.engine.run_tick(&self.graph);
            self.ingest_tick(tick);
        }

        for (node, handle) in &self.strategies {
            ui.separator();
            ui.label(format!("strategy #{}", node.0));

            // Live A/B: select a variant immediately (atomic, no recompile).
            let sel = handle.selected();
            ui.horizontal_wrapped(|ui| {
                for v in 0..handle.variant_count() {
                    if ui
                        .selectable_label(sel == v, handle.variant_name(v))
                        .clicked()
                    {
                        handle.select(v);
                    }
                }
            });

            if let Some(res) = self.tune_results.get(&node.0) {
                let max = res
                    .per_variant_best
                    .iter()
                    .max()
                    .copied()
                    .unwrap_or_default()
                    .as_secs_f64()
                    .max(1e-9);
                for v in 0..handle.variant_count() {
                    let d = res.per_variant_best.get(v).copied().unwrap_or_default();
                    let rejected = res.rejected.contains(&v);
                    let chosen = res.chosen == v;
                    let (tag, color) = if rejected {
                        ("✗ ≠ ref", Color32::from_rgb(220, 100, 100))
                    } else if chosen {
                        ("✓ chosen", Color32::from_rgb(232, 194, 64))
                    } else {
                        ("✓ verified", Color32::from_gray(150))
                    };
                    let frac = (d.as_secs_f64() / max) as f32;
                    ui.add(egui::ProgressBar::new(frac).fill(color).text(format!(
                        "{}  {}  {tag}",
                        handle.variant_name(v),
                        fmt_dur(d)
                    )));
                }
            } else {
                ui.weak("press Tune to benchmark variants");
            }
        }
    }
}
