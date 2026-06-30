//! Pure layered layout: assign each node a rectangle in world space, left→right by depth level.
//! No window/egui-context needed (`Rect`/`Pos2` are plain math), so this is unit-tested in CI.

use crate::model::{ViewGraph, ViewNode};
use eframe::egui::{pos2, vec2, Rect};

pub const NODE_W: f32 = 180.0;
pub const TITLE_H: f32 = 22.0;
pub const PIN_ROW: f32 = 18.0;
pub const PAD: f32 = 10.0;
pub const COL_GAP: f32 = 90.0;
pub const ROW_GAP: f32 = 28.0;

/// World-space rectangles, index-aligned with `ViewGraph::nodes` (i.e. `NodeId.0`).
pub struct Layout {
    pub rects: Vec<Rect>,
}

/// Box height: a title row plus one row per pin (whichever side has more), plus padding.
pub fn node_height(n: &ViewNode) -> f32 {
    let rows = n.inputs.len().max(n.outputs.len()).max(1) as f32;
    TITLE_H + rows * PIN_ROW + PAD
}

/// Longest-path depth level per node (same recurrence the scheduler uses), then pack each level
/// into a left→right column.
pub fn layout(view: &ViewGraph) -> Layout {
    let n = view.nodes.len();
    let mut level = vec![0usize; n];
    // Relax over all edges n times (graphs are small; this converges to the longest path).
    for _ in 0..n {
        for e in &view.edges {
            let cand = level[e.from.0] + 1;
            if cand > level[e.to.0] {
                level[e.to.0] = cand;
            }
        }
    }

    let max_level = level.iter().copied().max().unwrap_or(0);
    let mut rects = vec![Rect::ZERO; n];
    let mut col_x = 0.0f32;
    for l in 0..=max_level {
        let mut y = 0.0f32;
        for i in (0..n).filter(|&i| level[i] == l) {
            let h = node_height(&view.nodes[i]);
            rects[i] = Rect::from_min_size(pos2(col_x, y), vec2(NODE_W, h));
            y += h + ROW_GAP;
        }
        col_x += NODE_W + COL_GAP;
    }
    Layout { rects }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::SceneKind;

    #[test]
    fn diagnostics_lays_out_left_to_right_by_level() {
        let (graph, _) = SceneKind::Diagnostics.build();
        let view = ViewGraph::from_graph(&graph);
        let lay = layout(&view);

        assert_eq!(lay.rects.len(), graph.node_count());
        // cam(0) → threshold(1) → blobcount(2) → probe(3) → {log(4), logfmt(5)}
        let x = |i: usize| lay.rects[i].left();
        assert!(x(0) < x(1), "camera left of threshold");
        assert!(x(1) < x(2), "threshold left of blobcount");
        assert!(x(2) < x(3), "blobcount left of probe");
        assert!(x(3) < x(4), "probe left of log");
        // log and logfmt are both fed by the probe → same column.
        assert_eq!(x(4), x(5), "the two sinks share a column");
    }
}
