//! Property editors for a node's JSON config (`Node::to_json`).
//!
//! Two tiers: [`schema_editor`] renders real widgets (sliders for ranged numbers, checkboxes,
//! per-component vector drags, doc tooltips) driven by the node's [`ParamSchema`] — the one
//! `#[derive(NodeParams)]` produces from the struct; [`json_editor`] is the generic recursive
//! fallback for nodes without a schema. Both return whether anything changed this frame; the
//! caller rebuilds the node from the edited JSON via a serde factory.

use eframe::egui::{self, DragValue};
use octans_core::{ParamKind, ParamSchema};
use serde_json::Value;

/// Schema-driven property panel: one labeled, doc-tooltipped, kind-appropriate widget per field.
pub(crate) fn schema_editor(ui: &mut egui::Ui, schema: &ParamSchema, value: &mut Value) -> bool {
    let Some(obj) = value.as_object_mut() else {
        return json_editor(ui, value);
    };
    let mut changed = false;
    egui::Grid::new("param-grid").num_columns(2).show(ui, |ui| {
        for f in &schema.fields {
            let Some(v) = obj.get_mut(f.name) else {
                continue;
            };
            let label = ui.label(f.name);
            if !f.doc.is_empty() {
                label.on_hover_text(f.doc);
            }
            changed |= field_widget(ui, &f.kind, v);
            ui.end_row();
        }
    });
    changed
}

fn field_widget(ui: &mut egui::Ui, kind: &ParamKind, v: &mut Value) -> bool {
    match kind {
        ParamKind::Bool => {
            let mut b = v.as_bool().unwrap_or(false);
            let r = ui.checkbox(&mut b, "");
            if r.changed() {
                *v = Value::Bool(b);
            }
            r.changed()
        }
        ParamKind::Int { min, max } => int_widget(ui, v, *min, *max),
        ParamKind::Float { min, max } => float_widget(ui, v, *min, *max),
        ParamKind::Text => {
            let mut s = v.as_str().unwrap_or("").to_string();
            let r = ui.text_edit_singleline(&mut s);
            if r.changed() {
                *v = Value::String(s);
            }
            r.changed()
        }
        ParamKind::FloatArray { .. } => {
            let Some(arr) = v.as_array_mut() else {
                return json_editor(ui, v);
            };
            let mut changed = false;
            ui.horizontal(|ui| {
                for elem in arr.iter_mut() {
                    let mut x = elem.as_f64().unwrap_or(0.0);
                    if ui.add(DragValue::new(&mut x).speed(0.05)).changed() {
                        if let Some(n) = serde_json::Number::from_f64(x) {
                            *elem = Value::Number(n);
                            changed = true;
                        }
                    }
                }
            });
            changed
        }
        ParamKind::Json => json_editor(ui, v),
    }
}

/// Integer widget preserving the JSON value's u64/i64 flavor; a slider when fully bounded.
fn int_widget(ui: &mut egui::Ui, v: &mut Value, min: Option<f64>, max: Option<f64>) -> bool {
    if v.is_u64() {
        let mut x = v.as_u64().unwrap_or(0);
        let changed = match (min, max) {
            (Some(lo), Some(hi)) => ui
                .add(egui::Slider::new(&mut x, lo.max(0.0) as u64..=hi as u64))
                .changed(),
            _ => ui
                .add(DragValue::new(&mut x).range(min.unwrap_or(0.0)..=max.unwrap_or(f64::MAX)))
                .changed(),
        };
        if changed {
            *v = Value::from(x);
        }
        changed
    } else {
        let mut x = v.as_i64().unwrap_or(0);
        let changed = match (min, max) {
            (Some(lo), Some(hi)) => ui
                .add(egui::Slider::new(&mut x, lo as i64..=hi as i64))
                .changed(),
            _ => ui
                .add(
                    DragValue::new(&mut x).range(min.unwrap_or(f64::MIN)..=max.unwrap_or(f64::MAX)),
                )
                .changed(),
        };
        if changed {
            *v = Value::from(x);
        }
        changed
    }
}

fn float_widget(ui: &mut egui::Ui, v: &mut Value, min: Option<f64>, max: Option<f64>) -> bool {
    let mut x = v.as_f64().unwrap_or(0.0);
    let changed = match (min, max) {
        (Some(lo), Some(hi)) => ui.add(egui::Slider::new(&mut x, lo..=hi)).changed(),
        _ => ui
            .add(
                DragValue::new(&mut x)
                    .speed(0.5)
                    .range(min.unwrap_or(f64::MIN)..=max.unwrap_or(f64::MAX)),
            )
            .changed(),
    };
    if changed {
        if let Some(n) = serde_json::Number::from_f64(x) {
            *v = Value::Number(n);
        }
    }
    changed
}

/// Edit a JSON value in place. Returns `true` if the user changed it this frame.
pub(crate) fn json_editor(ui: &mut egui::Ui, value: &mut Value) -> bool {
    match value {
        Value::Object(map) => {
            let mut changed = false;
            for (k, v) in map.iter_mut() {
                let inner = ui.horizontal(|ui| {
                    ui.label(k.as_str());
                    json_editor(ui, v)
                });
                changed |= inner.inner;
            }
            changed
        }
        Value::Array(arr) => {
            let inner = ui.horizontal_wrapped(|ui| {
                let mut c = false;
                for v in arr.iter_mut() {
                    c |= json_editor(ui, v);
                }
                c
            });
            inner.inner
        }
        Value::Bool(b) => ui.checkbox(b, "").changed(),
        Value::String(s) => ui.text_edit_singleline(s).changed(),
        Value::Number(_) => edit_number(ui, value),
        Value::Null => {
            ui.weak("null");
            false
        }
    }
}

/// Edit a JSON number, preserving its integer/float flavor (so it still deserializes correctly).
fn edit_number(ui: &mut egui::Ui, value: &mut Value) -> bool {
    if value.is_u64() {
        let mut x = value.as_u64().unwrap_or(0);
        let r = ui.add(DragValue::new(&mut x));
        if r.changed() {
            *value = Value::from(x);
        }
        r.changed()
    } else if value.is_i64() {
        let mut x = value.as_i64().unwrap_or(0);
        let r = ui.add(DragValue::new(&mut x));
        if r.changed() {
            *value = Value::from(x);
        }
        r.changed()
    } else {
        let mut x = value.as_f64().unwrap_or(0.0);
        let r = ui.add(DragValue::new(&mut x).speed(0.5));
        if r.changed() {
            if let Some(n) = serde_json::Number::from_f64(x) {
                *value = Value::Number(n);
            }
        }
        r.changed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn number_flavors_are_preserved_by_from() {
        // These are the conversions edit_number relies on; they must keep int/float typing.
        assert!(Value::from(64u64).is_u64());
        assert!(Value::from(-3i64).is_i64());
        assert!(Value::Number(serde_json::Number::from_f64(400.0).unwrap()).is_f64());
    }
}
