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

/// Constructs a fresh node instance of a catalog type.
type NodeFactory = Box<dyn Fn() -> Box<dyn Node> + Send + Sync>;

/// One node type's description for the palette, plus a factory to construct it.
pub struct NodeClass {
    pub type_id: &'static str,
    pub display_name: String,
    pub category: String,
    /// Input ports: `(name, type, optional)`.
    pub inputs: Vec<(String, TypeSpec, bool)>,
    /// Output ports: `(name, type)`.
    pub outputs: Vec<(String, TypeSpec)>,
    factory: NodeFactory,
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

    /// Register a node type from a constructor. A sample is built once to read its
    /// `node_type()`/`inputs()`/`outputs()` (display name + category derived from the type id,
    /// e.g. `octans.std.threshold` → category `std`, name `threshold`); the constructor is kept to
    /// build fresh instances on demand (see [`make`](Catalog::make)).
    pub fn add<N, F>(&mut self, make: F)
    where
        N: Node + 'static,
        F: Fn() -> N + Send + Sync + 'static,
    {
        let sample = make();
        let id = sample.node_type();
        let segs: Vec<&str> = id.split('.').collect();
        let category = if segs.len() >= 3 { segs[1] } else { "misc" }.to_string();
        let display_name = id.rsplit('.').next().unwrap_or(id).replace('_', " ");
        let inputs = sample
            .inputs()
            .into_iter()
            .map(|p| (p.name.to_string(), p.ty, p.optional))
            .collect();
        let outputs = sample
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
            factory: Box::new(move || Box::new(make()) as Box<dyn Node>),
        });
    }

    /// Construct a fresh node of the given type, if it's registered.
    pub fn make(&self, type_id: &str) -> Option<Box<dyn Node>> {
        self.get(type_id).map(|c| (c.factory)())
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
