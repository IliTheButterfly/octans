//! `Mira` — the interpreter engine.
//!
//! Named for Mira, the famous *variable* star: this is the engine that runs a graph which is
//! constantly changing under the author's edits (the "live/edit" tier, analogous to the
//! lightest model in a tier set). It compiles a topological order once, then runs ticks.
//!
//! v0 is single-threaded and re-runs the whole order each tick. The topological order already
//! *contains* independent same-depth levels; a later engine (`Vega` JIT, `Canopus` AOT) is
//! where those levels get scheduled across threads / GPU. The seam is deliberately here.

use crate::graph::{Graph, NodeId};
use crate::node::{Inputs, Outputs};
use crate::value::Value;
use std::collections::HashMap;
use std::time::{Duration, Instant};

pub struct Mira {
    order: Vec<usize>,
}

#[derive(Debug)]
pub enum CompileError {
    /// The graph has a cycle (dataflow cycles must go through a portal/feedback edge, which
    /// is not yet implemented in v0).
    Cycle,
}

impl Mira {
    /// Compile the graph into an execution plan (Kahn topological sort).
    pub fn compile(graph: &Graph) -> Result<Self, CompileError> {
        let n = graph.nodes.len();
        let mut indeg = vec![0usize; n];
        let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
        for e in &graph.edges {
            adj[e.from_node].push(e.to_node);
            indeg[e.to_node] += 1;
        }
        let mut order = Vec::with_capacity(n);
        let mut queue: Vec<usize> = (0..n).filter(|&i| indeg[i] == 0).collect();
        let mut head = 0;
        while head < queue.len() {
            let u = queue[head];
            head += 1;
            order.push(u);
            for &v in &adj[u] {
                indeg[v] -= 1;
                if indeg[v] == 0 {
                    queue.push(v);
                }
            }
        }
        if order.len() != n {
            return Err(CompileError::Cycle);
        }
        Ok(Self { order })
    }

    /// Run one tick (one frame) through the whole graph; returns its wall-clock latency.
    pub fn run_tick(&self, graph: &Graph) -> Tick {
        let start = Instant::now();
        // (node, output-port) -> latest value produced this tick.
        let mut store: HashMap<(usize, &'static str), Value> = HashMap::new();

        for &nid in &self.order {
            // Gather inputs by following the edges that feed this node. We clone the `Value`s
            // (cheap Arc clones — still zero-copy) so the node holds no borrow into `store`.
            let mut inmap: HashMap<&'static str, Value> = HashMap::new();
            for e in &graph.edges {
                if e.to_node == nid {
                    if let Some(v) = store.get(&(e.from_node, e.from_port)) {
                        inmap.insert(e.to_port, v.clone());
                    }
                }
            }
            // Unconnected input ports fall back to their default (parameter behaviour).
            for spec in graph.nodes[nid].inputs() {
                if !inmap.contains_key(spec.name) {
                    if let Some(d) = spec.default {
                        inmap.insert(spec.name, d);
                    }
                }
            }

            let inputs = Inputs { map: inmap };
            let mut outputs = Outputs::default();
            graph.nodes[nid].process(&inputs, &mut outputs);

            for (port, val) in outputs.map {
                store.insert((nid, port), val);
            }
        }

        Tick { latency: start.elapsed(), store }
    }
}

/// The result of one tick: latency + the values each node produced (for inspection/sinks).
pub struct Tick {
    pub latency: Duration,
    store: HashMap<(usize, &'static str), Value>,
}

impl Tick {
    pub fn output(&self, node: NodeId, port: &'static str) -> Option<&Value> {
        self.store.get(&(node.0, port))
    }
}
