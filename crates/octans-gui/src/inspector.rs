//! The Inspector — click a node to open a floating window showing its current outputs: `Image`s
//! rendered as textures, numeric values (`Pt3`/`Px`/scalars) read out, with a sparkline of each
//! component over recent ticks.

use crate::model::type_label;
use crate::OctansApp;
use eframe::egui::{self, Color32};
use octans_core::Value;
use octans_nodes::{Image, Pt3, Px};
use std::collections::VecDeque;

/// Extract a value's numeric components, if it's a type we can plot (for the sparklines/history).
pub(crate) fn scalar_components(v: &Value) -> Option<Vec<f64>> {
    if let Some(p) = v.downcast_ref::<Pt3>() {
        return Some(p.0.to_vec());
    }
    if let Some(p) = v.downcast_ref::<Px>() {
        return Some(p.0.to_vec());
    }
    macro_rules! prim {
        ($($t:ty),*) => {$(
            if let Some(x) = v.downcast_ref::<$t>() { return Some(vec![*x as f64]); }
        )*};
    }
    prim!(u8, u16, u32, u64, i32, i64, f32, f64);
    None
}

fn short(label: &str) -> &str {
    label.rsplit('.').next().unwrap_or(label)
}

fn fmt_components(c: &[f64]) -> String {
    if c.len() == 1 {
        format!("{:.4}", c[0])
    } else {
        let parts: Vec<String> = c.iter().map(|v| format!("{v:.4}")).collect();
        format!("[{}]", parts.join(", "))
    }
}

fn component_color(i: usize) -> Color32 {
    match i {
        0 => Color32::from_rgb(230, 120, 120),
        1 => Color32::from_rgb(120, 210, 130),
        2 => Color32::from_rgb(120, 160, 240),
        _ => Color32::from_gray(200),
    }
}

/// A tiny dependency-free sparkline: one normalized polyline per component over recent ticks.
fn sparkline(ui: &mut egui::Ui, hist: &VecDeque<Vec<f64>>) {
    let n = hist.len();
    if n < 2 {
        return;
    }
    let ncomp = hist.back().map(|c| c.len()).unwrap_or(0);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(240.0, 40.0), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, egui::CornerRadius::same(2), Color32::from_gray(28));
    for c in 0..ncomp {
        let vals: Vec<f64> = hist
            .iter()
            .map(|v| v.get(c).copied().unwrap_or(0.0))
            .collect();
        let mn = vals.iter().copied().fold(f64::INFINITY, f64::min);
        let mx = vals.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let span = (mx - mn).max(1e-9);
        let pts: Vec<egui::Pos2> = vals
            .iter()
            .enumerate()
            .map(|(i, &v)| {
                let x = rect.left() + rect.width() * (i as f32 / (n - 1) as f32);
                let y = rect.bottom() - rect.height() * ((v - mn) / span) as f32;
                egui::pos2(x, y)
            })
            .collect();
        painter.add(egui::Shape::line(
            pts,
            egui::Stroke::new(1.0, component_color(c)),
        ));
    }
}

impl OctansApp {
    fn show_image(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        key: (usize, &'static str),
        img: &Image,
    ) {
        let color = egui::ColorImage {
            size: [img.w, img.h],
            pixels: img.px.iter().map(|&g| Color32::from_gray(g)).collect(),
        };
        let opts = egui::TextureOptions::NEAREST;
        let tc = self.tick_count;
        let name = format!("img-{}-{}", key.0, key.1);
        match self.textures.get_mut(&key) {
            Some(e) if e.0 == tc => {}
            Some(e) => {
                e.1.set(color, opts);
                e.0 = tc;
            }
            None => {
                let h = ctx.load_texture(name, color, opts);
                self.textures.insert(key, (tc, h));
            }
        }
        let handle = &self.textures[&key].1;
        let st = egui::load::SizedTexture::new(handle.id(), handle.size_vec2());
        ui.add(egui::Image::new(st).max_width(240.0));
        ui.weak(format!("{}×{}", img.w, img.h));
    }

    pub(crate) fn inspector_window(&mut self, ctx: &egui::Context) {
        let Some(sel) = self.selected else {
            return;
        };
        // Read the node's identity/ports/config, then drop the borrow so the window closure can
        // take `&mut self` (texture cache, param edits, etc.).
        let (title, type_id, cfg, outs, schema) = {
            let Some(node) = self.graph.node(sel) else {
                self.selected = None;
                return;
            };
            (
                format!("inspector · {} #{}", short(node.node_type()), sel.0),
                node.node_type(),
                node.to_json(),
                node.outputs(),
                node.param_schema(),
            )
        };
        // (Re)load the editable config when the selection changes.
        if self.param_edit.as_ref().map(|(n, _)| *n) != Some(sel.0) {
            self.param_edit = Some((sel.0, cfg.clone()));
        }

        let mut open = true;
        let mut delete = false;
        let mut params_changed = false;
        egui::Window::new(title)
            .open(&mut open)
            .default_width(280.0)
            .resizable(true)
            .show(ctx, |ui| {
                if ui
                    .button("🗑 delete node")
                    .on_hover_text("or press Delete")
                    .clicked()
                {
                    delete = true;
                }
                ui.separator();

                // Parameters (editable config): schema-driven widgets when the node has a
                // `ParamSchema` (docs as tooltips, sliders for ranged fields), generic JSON
                // editor otherwise.
                if let Some((_, val)) = self.param_edit.as_mut() {
                    let empty =
                        val.is_null() || val.as_object().map(|o| o.is_empty()).unwrap_or(false);
                    if empty {
                        ui.weak("no editable parameters");
                    } else {
                        ui.strong("parameters");
                        params_changed = match &schema {
                            Some(s) => crate::params::schema_editor(ui, s, val),
                            None => crate::params::json_editor(ui, val),
                        };
                    }
                    ui.separator();
                }

                if outs.is_empty() {
                    ui.weak("this node has no outputs (a sink)");
                    return;
                }
                for p in &outs {
                    ui.separator();
                    ui.strong(format!("{} : {}", p.name, type_label(&p.ty)));
                    let key = (sel.0, p.name);
                    let Some(val) = self.values.get(&key).cloned() else {
                        ui.weak("— not produced this tick");
                        continue;
                    };
                    if let Some(img) = val.downcast_ref::<Image>() {
                        self.show_image(ui, ctx, key, img);
                    } else if let Some(comps) = scalar_components(&val) {
                        ui.monospace(fmt_components(&comps));
                        if let Some(h) = self.history.get(&key) {
                            sparkline(ui, h);
                        }
                    } else {
                        ui.weak("(no preview for this type)");
                    }
                }
            });
        if delete {
            self.delete_node(sel);
            return;
        }
        // Apply a parameter edit: rebuild the node from the edited config and swap it in (ports
        // unchanged, so a light recompile preserves view/layout/selection). Consecutive edits to
        // the same node coalesce into one undo step (a slider drag isn't 60 undos).
        if params_changed {
            if let Some((_, val)) = &self.param_edit {
                let after = val.clone();
                if let Some(node) = self.node_registry.build(type_id, &after) {
                    use crate::history::EditAction;
                    if let Some(EditAction::ParamEdit { id, after: a, .. }) =
                        self.undo_stack.last_mut()
                    {
                        if *id == sel.0 {
                            *a = after.clone();
                            self.redo_stack.clear();
                        } else {
                            self.push_edit(EditAction::ParamEdit {
                                id: sel.0,
                                type_id: type_id.to_string(),
                                before: cfg.clone(),
                                after: after.clone(),
                            });
                        }
                    } else {
                        self.push_edit(EditAction::ParamEdit {
                            id: sel.0,
                            type_id: type_id.to_string(),
                            before: cfg.clone(),
                            after: after.clone(),
                        });
                    }
                    self.graph.replace_node(sel, node);
                    self.recompile();
                }
            }
        }
        if !open {
            self.selected = None;
            self.param_edit = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scalar_components_handles_known_types() {
        assert_eq!(scalar_components(&Value::new(42u32)), Some(vec![42.0]));
        assert_eq!(scalar_components(&Value::new(1.5f64)), Some(vec![1.5]));
        assert_eq!(
            scalar_components(&Value::new(Pt3([1.0, 2.0, 3.0]))),
            Some(vec![1.0, 2.0, 3.0])
        );
        assert_eq!(
            scalar_components(&Value::new(Px([0.5, 0.25]))),
            Some(vec![0.5, 0.25])
        );
        // images aren't scalar — no sparkline
        assert!(scalar_components(&Value::new(Image {
            w: 1,
            h: 1,
            px: vec![0]
        }))
        .is_none());
    }
}
