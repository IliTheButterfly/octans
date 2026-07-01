//! A small generic editor for a node's JSON config (`Node::to_json`). Renders numbers, bools,
//! strings, arrays and objects recursively; returns whether anything changed this frame. The
//! caller rebuilds the node from the edited JSON via a serde factory.

use eframe::egui::{self, DragValue};
use serde_json::Value;

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
