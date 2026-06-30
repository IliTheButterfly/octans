//! `ViewGraph` — a structural snapshot of a live `Graph`, built by introspection.
//!
//! This is deliberately the *shape* a future editor will mutate (nodes with labels + typed ports,
//! plus edges), so the read-only viewer and the eventual editor share one model. The viewer simply
//! never mutates it.

use octans_core::{Graph, NodeId, Shape, TypeSpec, TOMBSTONE_TYPE};

/// One port (input or output) as the GUI needs it: a name and a human type label.
#[derive(Clone, Debug)]
pub struct PortInfo {
    pub name: String,
    pub ty: String,
    pub optional: bool,
}

/// A node as the GUI draws it.
#[derive(Clone, Debug)]
pub struct ViewNode {
    pub id: NodeId,
    pub label: String,
    pub inputs: Vec<PortInfo>,
    pub outputs: Vec<PortInfo>,
    /// A removed node's tombstone — kept for index alignment, but the GUI hides it.
    pub dead: bool,
}

/// A connection as the GUI draws it.
#[derive(Clone, Debug)]
pub struct ViewEdge {
    pub from: NodeId,
    pub from_port: String,
    pub to: NodeId,
    pub to_port: String,
}

/// A structural snapshot of a graph — nodes (index-aligned with `NodeId`) and edges.
#[derive(Clone, Debug, Default)]
pub struct ViewGraph {
    pub nodes: Vec<ViewNode>,
    pub edges: Vec<ViewEdge>,
}

/// A short, human label for a port type: the last `.`-segment of the type id plus a shape suffix
/// (`[]` for an unsized vector, `[N]` for a fixed one). E.g. `octans.std.image` -> `image`.
pub fn type_label(ty: &TypeSpec) -> String {
    let short = ty.id.rsplit('.').next().unwrap_or(ty.id);
    match ty.shape {
        Shape::Scalar => short.to_string(),
        Shape::Vector(None) => format!("{short}[]"),
        Shape::Vector(Some(n)) => format!("{short}[{n}]"),
    }
}

impl ViewGraph {
    /// Build a snapshot from a live graph using the public introspection API.
    pub fn from_graph(graph: &Graph) -> Self {
        let nodes = graph
            .nodes()
            .map(|(id, node)| ViewNode {
                id,
                label: node.node_type().to_string(),
                inputs: node
                    .inputs()
                    .iter()
                    .map(|p| PortInfo {
                        name: p.name.to_string(),
                        ty: type_label(&p.ty),
                        optional: p.optional,
                    })
                    .collect(),
                outputs: node
                    .outputs()
                    .iter()
                    .map(|p| PortInfo {
                        name: p.name.to_string(),
                        ty: type_label(&p.ty),
                        optional: p.optional,
                    })
                    .collect(),
                dead: node.node_type() == TOMBSTONE_TYPE,
            })
            .collect();

        let edges = graph
            .edges()
            .map(|e| ViewEdge {
                from: e.from,
                from_port: e.from_port.to_string(),
                to: e.to,
                to_port: e.to_port.to_string(),
            })
            .collect();

        Self { nodes, edges }
    }
}
