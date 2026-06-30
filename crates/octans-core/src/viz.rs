//! Headless visualization: render a graph to Graphviz **DOT**.
//!
//! No window required — this is a textual visual you can generate anywhere and render with
//! `dot -Tsvg graph.dot > graph.svg` (or any Graphviz viewer). Each node is a record showing
//! its type and ports; edges connect output ports to input ports. The interactive egui editor
//! is a later, display-bound effort; this is the verifiable stand-in.

use crate::graph::Graph;

fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(c, '"' | '\\' | '{' | '}' | '|' | '<' | '>') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

impl Graph {
    /// Render this graph as Graphviz DOT (record nodes with ports; one edge per connection).
    pub fn to_dot(&self) -> String {
        let mut s = String::from("digraph octans {\n");
        s.push_str("  rankdir=LR;\n");
        s.push_str("  node [shape=record, fontname=\"monospace\"];\n");

        for (i, n) in self.nodes.iter().enumerate() {
            let ins: Vec<String> = n
                .inputs()
                .iter()
                .map(|p| format!("<in_{}> {}", esc(p.name), esc(p.name)))
                .collect();
            let outs: Vec<String> = n
                .outputs()
                .iter()
                .map(|p| format!("<out_{}> {}", esc(p.name), esc(p.name)))
                .collect();

            let mut cells = Vec::new();
            if !ins.is_empty() {
                cells.push(format!("{{{}}}", ins.join("|")));
            }
            cells.push(format!("{} #{}", esc(n.node_type()), i));
            if !outs.is_empty() {
                cells.push(format!("{{{}}}", outs.join("|")));
            }
            s.push_str(&format!("  n{i} [label=\"{{{}}}\"];\n", cells.join("|")));
        }

        for e in &self.edges {
            s.push_str(&format!(
                "  n{}:out_{} -> n{}:in_{};\n",
                e.from_node, e.from_port, e.to_node, e.to_port
            ));
        }

        s.push_str("}\n");
        s
    }
}
