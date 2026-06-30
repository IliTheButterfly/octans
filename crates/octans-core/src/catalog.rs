//! The node catalog — an enumeration of available node *types* (not instances): their stable id,
//! a display name + category, and their port signature. This is what a GUI palette browses, and
//! the seam the future editor builds on (drag-to-add, then a construction factory).
//!
//! v1 derives a class from a *sample instance* (the registrar provides one), reading
//! `node_type()`/`inputs()`/`outputs()`. That needs no `Default` bound and no macro changes; a
//! later pass can have `#[node]` emit classes + construction factories directly.

use crate::node::Node;
use crate::value::TypeSpec;
use std::collections::BTreeMap;

/// One node type's description for the palette.
pub struct NodeClass {
    pub type_id: &'static str,
    pub display_name: String,
    pub category: String,
    /// Input ports: `(name, type, optional)`.
    pub inputs: Vec<(String, TypeSpec, bool)>,
    /// Output ports: `(name, type)`.
    pub outputs: Vec<(String, TypeSpec)>,
}

/// A set of node classes, grouped for browsing.
#[derive(Default)]
pub struct Catalog {
    classes: Vec<NodeClass>,
}

impl Catalog {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a node type from a sample instance. Display name and category are derived from the
    /// type id (`octans.std.threshold` → category `std`, name `threshold`); ports are read off the
    /// instance.
    pub fn add(&mut self, node: &dyn Node) {
        let id = node.node_type();
        let segs: Vec<&str> = id.split('.').collect();
        let category = if segs.len() >= 3 { segs[1] } else { "misc" }.to_string();
        let display_name = id.rsplit('.').next().unwrap_or(id).replace('_', " ");
        let inputs = node
            .inputs()
            .into_iter()
            .map(|p| (p.name.to_string(), p.ty, p.optional))
            .collect();
        let outputs = node
            .outputs()
            .into_iter()
            .map(|p| (p.name.to_string(), p.ty))
            .collect();
        self.classes.push(NodeClass {
            type_id: id,
            display_name,
            category,
            inputs,
            outputs,
        });
    }

    pub fn iter(&self) -> impl Iterator<Item = &NodeClass> {
        self.classes.iter()
    }

    pub fn len(&self) -> usize {
        self.classes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.classes.is_empty()
    }

    pub fn get(&self, type_id: &str) -> Option<&NodeClass> {
        self.classes.iter().find(|c| c.type_id == type_id)
    }

    /// Classes grouped by category (sorted), for a palette tree.
    pub fn by_category(&self) -> BTreeMap<&str, Vec<&NodeClass>> {
        let mut m: BTreeMap<&str, Vec<&NodeClass>> = BTreeMap::new();
        for c in &self.classes {
            m.entry(c.category.as_str()).or_default().push(c);
        }
        m
    }
}
