//! Parameter schemas — how a node describes its editable properties to a UI.
//!
//! A [`ParamSchema`] lists a node's config fields with enough metadata (kind, range, doc) for a
//! property panel to render real widgets — a slider for a ranged number, a checkbox for a bool —
//! instead of a raw JSON editor. Authors don't write schemas by hand: `#[derive(NodeParams)]`
//! (octans-macros) derives one from the struct's fields, field types, doc comments, and
//! `#[param(min = …, max = …)]` attributes, and `#[node(params)]` wires it into
//! [`Node::param_schema`](crate::Node::param_schema).

/// A node's editable parameters, in field-declaration order.
#[derive(Clone, Debug, PartialEq)]
pub struct ParamSchema {
    pub fields: Vec<ParamField>,
}

/// One editable field.
#[derive(Clone, Debug, PartialEq)]
pub struct ParamField {
    pub name: &'static str,
    /// The field's doc comment (shown as a tooltip). Empty if undocumented.
    pub doc: &'static str,
    pub kind: ParamKind,
}

/// What widget a field wants. Ranges are `f64` regardless of the field's integer width; the
/// JSON value's own int/float flavor drives (de)serialization.
#[derive(Clone, Debug, PartialEq)]
pub enum ParamKind {
    Bool,
    /// Any integer field (signed or unsigned — the JSON value knows which).
    Int {
        min: Option<f64>,
        max: Option<f64>,
    },
    Float {
        min: Option<f64>,
        max: Option<f64>,
    },
    Text,
    /// A fixed-size float array (positions, velocities): `[f64; N]` / `[f32; N]`.
    FloatArray {
        len: usize,
    },
    /// Anything else — rendered by a generic JSON editor.
    Json,
}
