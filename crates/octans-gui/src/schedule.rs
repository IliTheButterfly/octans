//! The parallel-schedule view — Octans's level-parallel scheduler made visible.
//!
//! Nodes at the same depth level run concurrently; this panel shows how many run in each wave, the
//! per-level wall time (the slowest node in the wave, since they overlap), the serial-vs-parallel
//! speedup that parallelism buys, and (on the canvas) the **critical path** — the longest-latency
//! dependency chain, i.e. the bottleneck you'd target with a `Strategy`/optimization.

use crate::model::ViewGraph;
use crate::{fmt_dur, OctansApp};
use eframe::egui;
use octans_core::NodeId;
use std::time::Duration;

/// Per-level summary: how many nodes ran in parallel and the wave's wall time (its slowest node).
pub struct LevelInfo {
    pub level: usize,
    pub node_count: usize,
    pub max_last: Duration,
    /// Type label of the slowest node in the wave (the level's bottleneck).
    pub bottleneck: String,
}

pub struct ScheduleSummary {
    pub levels: Vec<LevelInfo>,
    /// Sum of every node's latency (a serial engine's tick time).
    pub serial: Duration,
    /// Sum of per-level maxima (this engine's barrier-synchronized parallel tick time).
    pub parallel: Duration,
}

fn short(label: &str) -> &str {
    label.rsplit('.').next().unwrap_or(label)
}

/// Summarize the schedule from per-node levels and latencies (both index-aligned with `NodeId.0`).
pub fn schedule_summary(
    view: &ViewGraph,
    levels: &[usize],
    latencies: &[Duration],
) -> ScheduleSummary {
    let max_level = levels.iter().copied().max().unwrap_or(0);
    let mut out = Vec::new();
    let mut serial = Duration::ZERO;
    let mut parallel = Duration::ZERO;
    for l in 0..=max_level {
        let members: Vec<usize> = (0..view.nodes.len())
            .filter(|&i| levels[i] == l && !view.nodes[i].dead)
            .collect();
        if members.is_empty() {
            continue;
        }
        let mut max_last = Duration::ZERO;
        let mut bottleneck = String::new();
        for &i in &members {
            serial += latencies[i];
            if latencies[i] >= max_last {
                max_last = latencies[i];
                bottleneck = short(&view.nodes[i].label).to_string();
            }
        }
        parallel += max_last;
        out.push(LevelInfo {
            level: l,
            node_count: members.len(),
            max_last,
            bottleneck,
        });
    }
    ScheduleSummary {
        levels: out,
        serial,
        parallel,
    }
}

/// The longest-latency dependency chain (source→sink). Highlighted on the canvas as the bottleneck.
pub fn critical_path(view: &ViewGraph, levels: &[usize], latencies: &[Duration]) -> Vec<NodeId> {
    let n = view.nodes.len();
    if n == 0 {
        return Vec::new();
    }
    let mut preds: Vec<Vec<usize>> = vec![Vec::new(); n];
    for e in &view.edges {
        preds[e.to.0].push(e.from.0);
    }
    // Process in ascending level order so every predecessor is finalized first.
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by_key(|&i| levels[i]);

    let mut cp = vec![Duration::ZERO; n];
    let mut best_pred = vec![usize::MAX; n];
    for &i in &order {
        let mut bt = Duration::ZERO;
        let mut bp = usize::MAX;
        for &p in &preds[i] {
            if cp[p] > bt {
                bt = cp[p];
                bp = p;
            }
        }
        cp[i] = latencies[i] + bt;
        best_pred[i] = bp;
    }
    let mut end = 0;
    let mut best = Duration::ZERO;
    for (i, &t) in cp.iter().enumerate() {
        if t >= best {
            best = t;
            end = i;
        }
    }
    let mut chain = Vec::new();
    let mut cur = end;
    while cur != usize::MAX {
        chain.push(NodeId(cur));
        cur = best_pred[cur];
    }
    chain.reverse();
    chain
}

impl OctansApp {
    pub(crate) fn schedule_ui(&mut self, ui: &mut egui::Ui) {
        ui.strong("Parallel schedule");
        ui.checkbox(&mut self.show_critical_path, "critical path");

        let lat = self.latencies();
        let sum = schedule_summary(&self.view, &self.layout.levels, &lat);

        if self.tick_count == 0 {
            ui.weak("run a tick to measure");
        } else {
            let speedup = if sum.parallel > Duration::ZERO {
                sum.serial.as_secs_f64() / sum.parallel.as_secs_f64()
            } else {
                1.0
            };
            ui.label(format!(
                "serial {} → parallel {}  ({speedup:.1}×)",
                fmt_dur(sum.serial),
                fmt_dur(sum.parallel)
            ));
        }
        ui.separator();

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for li in &sum.levels {
                    ui.horizontal(|ui| {
                        ui.monospace(format!("L{:<2}", li.level));
                        ui.label(format!("{}‖", li.node_count));
                        if self.tick_count > 0 {
                            ui.monospace(fmt_dur(li.max_last));
                            ui.weak(&li.bottleneck);
                        }
                    });
                }
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::node_levels;
    use crate::scene::SceneKind;

    #[test]
    fn critical_path_follows_the_slowest_chain() {
        let graph = SceneKind::Diagnostics.build().graph;
        let view = ViewGraph::from_graph(&graph);
        let levels = node_levels(&view);
        // Make node #2 (blobcount) dominate: it should be on the critical path.
        let mut lat = vec![Duration::from_micros(1); view.nodes.len()];
        lat[2] = Duration::from_micros(1000);
        let path = critical_path(&view, &levels, &lat);
        assert!(
            path.contains(&NodeId(2)),
            "critical path includes the hot node"
        );
        // Path is contiguous source→sink: levels strictly increase along it.
        let lv: Vec<usize> = path.iter().map(|n| levels[n.0]).collect();
        assert!(
            lv.windows(2).all(|w| w[0] < w[1]),
            "levels increase along the path"
        );
    }

    #[test]
    fn summary_parallel_le_serial() {
        let graph = SceneKind::Tracker.build().graph;
        let view = ViewGraph::from_graph(&graph);
        let levels = node_levels(&view);
        let lat = vec![Duration::from_micros(100); view.nodes.len()];
        let sum = schedule_summary(&view, &levels, &lat);
        assert!(
            sum.parallel <= sum.serial,
            "parallel wall time can't exceed serial"
        );
        assert!(!sum.levels.is_empty());
    }
}
