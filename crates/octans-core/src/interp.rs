//! `Mira` — the interpreter engine (the live/edit tier; named for the variable star, since it
//! runs a graph that's constantly changing under edits). Compiles a topological order once,
//! owns each node's per-instance local state, and runs timed ticks.
//!
//! v0 is single-threaded and re-runs the whole order each tick. The order already contains
//! independent same-depth levels; a later engine (`Vega` JIT, `Canopus` AOT) is where those
//! get scheduled across threads/GPU and where local state is replicated per lane. The seam is
//! deliberately here.

use crate::context::Context;
use crate::graph::{Graph, NodeId};
use crate::node::{Inputs, Outputs};
use crate::value::Value;
use std::any::Any;
use std::collections::HashMap;
use std::time::{Duration, Instant};

pub struct Mira {
    order: Vec<usize>,
    locals: Vec<Box<dyn Any + Send>>, // one per node instance (indexed by NodeId)
    ctx: Context,
}

#[derive(Debug)]
pub enum CompileError {
    /// The graph has a dataflow cycle (cycles must route through a portal, which is acyclic for
    /// scheduling).
    Cycle,
}

impl Mira {
    /// Compile the graph into an execution plan (Kahn topological sort) and allocate each node's
    /// initial local state.
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
        let locals = graph.nodes.iter().map(|node| node.new_local()).collect();
        Ok(Self {
            order,
            locals,
            ctx: Context::new(),
        })
    }

    /// Access the shared context to install resources before running (e.g. calibration tables).
    pub fn context_mut(&mut self) -> &mut Context {
        &mut self.ctx
    }

    /// Run one tick (one frame); returns its wall-clock latency.
    pub fn run_tick(&mut self, graph: &Graph) -> Tick {
        let start = Instant::now();
        let Mira { order, locals, ctx } = self;
        ctx.advance();

        // (node, output-port) -> latest value produced this tick.
        let mut store: HashMap<(usize, &'static str), Value> = HashMap::new();

        for &nid in order.iter() {
            // Gather inputs: connected edges first, then unconnected ports' defaults.
            let mut inmap: HashMap<&'static str, Value> = HashMap::new();
            for e in &graph.edges {
                if e.to_node == nid {
                    if let Some(v) = store.get(&(e.from_node, e.from_port)) {
                        inmap.insert(e.to_port, v.clone());
                    }
                }
            }
            for spec in graph.nodes[nid].inputs() {
                if !inmap.contains_key(spec.name) {
                    if let Some(d) = spec.default {
                        inmap.insert(spec.name, d);
                    }
                }
            }

            let inputs = Inputs { map: inmap };
            let mut outputs = Outputs::default();
            let local: &mut dyn Any = &mut *locals[nid];
            graph.nodes[nid].process(ctx, local, &inputs, &mut outputs);

            for (port, val) in outputs.map {
                store.insert((nid, port), val);
            }
        }

        // Tick boundary: promote each portal's write to be next tick's read.
        for portal in &graph.portals {
            portal.swap();
        }

        Tick {
            latency: start.elapsed(),
            store,
        }
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
