//! The Profiler panel: a sortable per-node latency table from `Mira::profile()`.

use crate::{OctansApp, ProfileKey, ProfileSort};
use eframe::egui;
use octans_core::NodeId;
use std::time::Duration;

/// One profiler row (owned — no borrow of the engine, so the table can be sorted freely).
pub struct Row {
    pub id: NodeId,
    pub ty: &'static str,
    pub last: Duration,
    pub mean: Duration,
    pub max: Duration,
    pub samples: u64,
}

/// Sort rows by the chosen key/direction. Pure → unit-tested.
pub fn sort_rows(rows: &mut [Row], sort: ProfileSort) {
    rows.sort_by(|a, b| {
        let ord = match sort.key {
            ProfileKey::Id => a.id.0.cmp(&b.id.0),
            ProfileKey::Type => a.ty.cmp(b.ty),
            ProfileKey::Last => a.last.cmp(&b.last),
            ProfileKey::Mean => a.mean.cmp(&b.mean),
            ProfileKey::Max => a.max.cmp(&b.max),
            ProfileKey::Samples => a.samples.cmp(&b.samples),
        };
        if sort.desc {
            ord.reverse()
        } else {
            ord
        }
    });
}

fn fmt_dur(d: Duration) -> String {
    let us = d.as_secs_f64() * 1e6;
    if us >= 1000.0 {
        format!("{:.2} ms", us / 1000.0)
    } else {
        format!("{us:.1} µs")
    }
}

impl OctansApp {
    pub(crate) fn profiler_ui(&mut self, ui: &mut egui::Ui) {
        ui.strong("Profiler");
        if let Some(t) = &self.last_tick {
            ui.label(format!(
                "tick total: {:.3} ms",
                t.latency.as_secs_f64() * 1e3
            ));
        }
        ui.separator();

        // Build owned rows (no engine borrow held past this point).
        let mut rows: Vec<Row> = self
            .engine
            .profile()
            .iter()
            .map(|(id, st)| Row {
                id,
                ty: self.graph.node(id).map(|n| n.node_type()).unwrap_or("?"),
                last: st.last,
                mean: st.mean(),
                max: st.max,
                samples: st.samples,
            })
            .collect();
        sort_rows(&mut rows, self.profiler_sort);

        let mut header = |ui: &mut egui::Ui, label: &str, key: ProfileKey| {
            let arrow = if self.profiler_sort.key == key {
                if self.profiler_sort.desc {
                    " ▼"
                } else {
                    " ▲"
                }
            } else {
                ""
            };
            if ui.button(format!("{label}{arrow}")).clicked() {
                if self.profiler_sort.key == key {
                    self.profiler_sort.desc = !self.profiler_sort.desc;
                } else {
                    self.profiler_sort = ProfileSort { key, desc: true };
                }
            }
        };

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                egui::Grid::new("profiler-grid")
                    .striped(true)
                    .num_columns(6)
                    .show(ui, |ui| {
                        header(ui, "node", ProfileKey::Id);
                        header(ui, "type", ProfileKey::Type);
                        header(ui, "last", ProfileKey::Last);
                        header(ui, "mean", ProfileKey::Mean);
                        header(ui, "max", ProfileKey::Max);
                        header(ui, "n", ProfileKey::Samples);
                        ui.end_row();

                        for r in &rows {
                            ui.monospace(format!("#{}", r.id.0));
                            ui.label(r.ty.rsplit('.').next().unwrap_or(r.ty));
                            ui.monospace(fmt_dur(r.last));
                            ui.monospace(fmt_dur(r.mean));
                            ui.monospace(fmt_dur(r.max));
                            ui.monospace(r.samples.to_string());
                            ui.end_row();
                        }
                    });
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: usize, mean_us: u64) -> Row {
        Row {
            id: NodeId(id),
            ty: "t",
            last: Duration::ZERO,
            mean: Duration::from_micros(mean_us),
            max: Duration::ZERO,
            samples: 0,
        }
    }

    #[test]
    fn sort_by_mean_desc_then_asc() {
        let mut rows = vec![row(0, 10), row(1, 30), row(2, 20)];
        sort_rows(
            &mut rows,
            ProfileSort {
                key: ProfileKey::Mean,
                desc: true,
            },
        );
        assert_eq!(rows.iter().map(|r| r.id.0).collect::<Vec<_>>(), [1, 2, 0]);
        sort_rows(
            &mut rows,
            ProfileSort {
                key: ProfileKey::Mean,
                desc: false,
            },
        );
        assert_eq!(rows.iter().map(|r| r.id.0).collect::<Vec<_>>(), [0, 2, 1]);
    }
}
