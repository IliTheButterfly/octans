//! The custom canvas: draws node boxes, typed pins, bezier edges, with pan/zoom and per-tick state
//! colors. Read-only for now, but the `Sense` is drag-ready for the future editor.

use crate::layout::{PIN_ROW, TITLE_H};
use crate::{OctansApp, TickSnapshot};
use eframe::egui::{self, pos2, Align2, Color32, CornerRadius, FontId, Pos2, Stroke, StrokeKind};
use octans_core::NodeId;
use std::collections::HashMap;

fn node_fill(id: NodeId, last: &Option<TickSnapshot>) -> Color32 {
    if let Some(t) = last {
        if t.faulted.contains(&id) {
            return Color32::from_rgb(120, 40, 40);
        }
        if t.skipped.contains(&id) {
            return Color32::from_gray(46);
        }
    }
    Color32::from_rgb(46, 52, 64)
}

fn node_stroke(id: NodeId, last: &Option<TickSnapshot>) -> Stroke {
    if let Some(t) = last {
        if t.faulted.contains(&id) {
            return Stroke::new(2.0, Color32::from_rgb(240, 90, 90));
        }
    }
    Stroke::new(1.0, Color32::from_gray(90))
}

/// Edge tint by its *source* node's state: a faulted/skipped source produced no data, so the
/// edge that would carry it is dimmed — making fault/skip cascades visible at a glance.
fn edge_color(from: NodeId, last: &Option<TickSnapshot>) -> Color32 {
    if let Some(t) = last {
        if t.faulted.contains(&from) {
            return Color32::from_rgb(90, 45, 45);
        }
        if t.skipped.contains(&from) {
            return Color32::from_gray(70);
        }
    }
    Color32::from_gray(150)
}

impl OctansApp {
    pub(crate) fn canvas_ui(&mut self, ui: &mut egui::Ui) {
        let (resp, painter) =
            ui.allocate_painter(ui.available_size(), egui::Sense::click_and_drag());
        let origin = resp.rect.min;

        // Pan with a background drag.
        if resp.dragged() {
            self.camera.pan += resp.drag_delta();
        }
        // Zoom on scroll, centered on the cursor.
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        if scroll != 0.0 {
            if let Some(ptr) = resp.hover_pos() {
                let old = self.camera.zoom;
                let new = (old * (1.0 + scroll * 0.001)).clamp(0.2, 4.0);
                let w = (ptr - origin - self.camera.pan) / old;
                self.camera.pan = (ptr - origin) - w * new;
                self.camera.zoom = new;
            }
        }

        painter.rect_filled(resp.rect, CornerRadius::ZERO, Color32::from_gray(20));

        let zoom = self.camera.zoom;
        let cam = self.camera;

        // Pin screen positions, keyed by (node index, port name).
        let mut out_pin: HashMap<(usize, String), Pos2> = HashMap::new();
        let mut in_pin: HashMap<(usize, String), Pos2> = HashMap::new();
        for vn in &self.view.nodes {
            let r = self.layout.rects[vn.id.0];
            for (j, p) in vn.inputs.iter().enumerate() {
                let wy = r.top() + TITLE_H + (j as f32 + 0.5) * PIN_ROW;
                in_pin.insert(
                    (vn.id.0, p.name.clone()),
                    cam.to_screen(pos2(r.left(), wy), origin),
                );
            }
            for (j, p) in vn.outputs.iter().enumerate() {
                let wy = r.top() + TITLE_H + (j as f32 + 0.5) * PIN_ROW;
                out_pin.insert(
                    (vn.id.0, p.name.clone()),
                    cam.to_screen(pos2(r.right(), wy), origin),
                );
            }
        }

        // Edges first (under the nodes).
        for e in &self.view.edges {
            if let (Some(&p0), Some(&p1)) = (
                out_pin.get(&(e.from.0, e.from_port.clone())),
                in_pin.get(&(e.to.0, e.to_port.clone())),
            ) {
                let dx = ((p1.x - p0.x).abs() * 0.5).max(30.0 * zoom);
                let bez = egui::epaint::CubicBezierShape::from_points_stroke(
                    [p0, pos2(p0.x + dx, p0.y), pos2(p1.x - dx, p1.y), p1],
                    false,
                    Color32::TRANSPARENT,
                    Stroke::new(1.5, edge_color(e.from, &self.last_tick)),
                );
                painter.add(bez);
            }
        }

        // Latency overlay normalization.
        let max_last = if self.show_latency_overlay {
            self.engine
                .profile()
                .iter()
                .map(|(_, s)| s.last.as_secs_f64())
                .fold(0.0_f64, f64::max)
                .max(1e-9)
        } else {
            0.0
        };

        // Nodes on top.
        let pin_font = FontId::monospace((10.0 * zoom).max(6.0));
        let title_font = FontId::monospace((12.0 * zoom).max(7.0));
        for vn in &self.view.nodes {
            let wr = self.layout.rects[vn.id.0];
            let r = cam.to_screen_rect(wr, origin);
            painter.rect_filled(r, CornerRadius::same(4), node_fill(vn.id, &self.last_tick));
            painter.rect_stroke(
                r,
                CornerRadius::same(4),
                node_stroke(vn.id, &self.last_tick),
                StrokeKind::Inside,
            );

            let faulted = self
                .last_tick
                .as_ref()
                .is_some_and(|t| t.faulted.contains(&vn.id));
            painter.text(
                cam.to_screen(pos2(wr.left() + 6.0, wr.top() + 3.0), origin),
                Align2::LEFT_TOP,
                format!(
                    "{}{} #{}",
                    if faulted { "⚠ " } else { "" },
                    short(&vn.label),
                    vn.id.0
                ),
                title_font.clone(),
                Color32::from_gray(235),
            );

            for (j, p) in vn.inputs.iter().enumerate() {
                let wy = wr.top() + TITLE_H + (j as f32 + 0.5) * PIN_ROW;
                let pin = cam.to_screen(pos2(wr.left(), wy), origin);
                painter.circle_filled(pin, 3.5 * zoom, Color32::from_gray(180));
                painter.text(
                    cam.to_screen(pos2(wr.left() + 10.0, wy), origin),
                    Align2::LEFT_CENTER,
                    format!("{}: {}", p.name, p.ty),
                    pin_font.clone(),
                    Color32::from_gray(190),
                );
            }
            for (j, p) in vn.outputs.iter().enumerate() {
                let wy = wr.top() + TITLE_H + (j as f32 + 0.5) * PIN_ROW;
                let pin = cam.to_screen(pos2(wr.right(), wy), origin);
                painter.circle_filled(pin, 3.5 * zoom, Color32::from_rgb(150, 200, 150));
                painter.text(
                    cam.to_screen(pos2(wr.right() - 10.0, wy), origin),
                    Align2::RIGHT_CENTER,
                    format!("{}: {}", p.name, p.ty),
                    pin_font.clone(),
                    Color32::from_gray(190),
                );
            }

            if self.show_latency_overlay {
                let last = self.engine.profile().node(vn.id).last.as_secs_f64();
                let frac = (last / max_last) as f32;
                painter.text(
                    cam.to_screen(pos2(wr.left() + 6.0, wr.bottom() - 14.0), origin),
                    Align2::LEFT_TOP,
                    format!("{:.0} µs", last * 1e6),
                    pin_font.clone(),
                    Color32::from_rgb(
                        (120.0 + 135.0 * frac) as u8,
                        (120.0 * (1.0 - frac)) as u8,
                        90,
                    ),
                );
            }

            // Hover for details: state (ok / faulted + message / skipped) and typed ports.
            let resp = ui.interact(r, egui::Id::new(("node", vn.id.0)), egui::Sense::click());
            if resp.hovered() {
                let last = &self.last_tick;
                resp.on_hover_ui(|ui| {
                    ui.strong(format!("{} #{}", vn.label, vn.id.0));
                    match last {
                        Some(t) if t.faulted.contains(&vn.id) => {
                            ui.colored_label(Color32::from_rgb(240, 90, 90), "FAULTED");
                            if let Some(f) = t.fault_msgs.iter().find(|f| f.node == vn.id) {
                                ui.label(&f.message);
                            }
                        }
                        Some(t) if t.skipped.contains(&vn.id) => {
                            ui.colored_label(
                                Color32::from_rgb(200, 200, 120),
                                "skipped — a required input was absent this tick",
                            );
                        }
                        Some(_) => {
                            ui.colored_label(Color32::from_rgb(140, 200, 140), "ok");
                        }
                        None => {
                            ui.weak("not run yet");
                        }
                    }
                    if !vn.inputs.is_empty() {
                        ui.separator();
                        ui.weak("inputs");
                        for p in &vn.inputs {
                            let opt = if p.optional { "  (optional)" } else { "" };
                            ui.monospace(format!("  {}: {}{}", p.name, p.ty, opt));
                        }
                    }
                    if !vn.outputs.is_empty() {
                        ui.weak("outputs");
                        for p in &vn.outputs {
                            ui.monospace(format!("  {}: {}", p.name, p.ty));
                        }
                    }
                });
            }
        }
    }
}

/// Last `.`-segment of a node-type id (`octans.std.threshold` → `threshold`).
fn short(label: &str) -> &str {
    label.rsplit('.').next().unwrap_or(label)
}
