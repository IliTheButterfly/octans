//! `Mira` — the interpreter engine (the live/edit tier; named for the variable star, since it
//! runs a graph that's constantly changing under edits). Compiles a topological order once,
//! owns each node's per-instance local state, and runs timed ticks.
//!
//! The per-tick pass is factored into [`run_order`] so it can be reused: `Map` runs a group
//! body's plan per lane through the same machinery.
//!
//! v0 is single-threaded at the top level. The order already contains independent same-depth
//! levels; a later engine (`Vega` JIT, `Canopus` AOT) is where those get scheduled across
//! threads/GPU. The seam is deliberately here.

use crate::context::Context;
use crate::graph::{Edge, Graph, NodeId};
use crate::node::{Inputs, Node, Outputs};
use crate::profile::Profile;
use crate::value::Value;
use std::any::Any;
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// A keyed map of values produced this tick: `(node index, output port) -> value`.
pub(crate) type Store = HashMap<(usize, &'static str), Value>;

/// Run one pass over `order`, returning every value produced. Inputs to each node come from
/// (1) `injected` boundary feeds, then (2) connected edges, then (3) unconnected ports'
/// defaults. `locals[i]` is node `i`'s private state (exclusive `&mut`).
pub(crate) fn run_order(
    nodes: &[Box<dyn Node>],
    edges: &[Edge],
    order: &[usize],
    locals: &mut [Box<dyn Any + Send>],
    ctx: &Context,
    injected: &HashMap<(usize, &'static str), Value>,
    timings: &mut Vec<(usize, Duration)>,
) -> Store {
    timings.clear();
    let mut store: Store = HashMap::new();

    for &nid in order {
        let mut inmap: HashMap<&'static str, Value> = HashMap::new();
        for e in edges {
            if e.to_node == nid {
                if let Some(v) = store.get(&(e.from_node, e.from_port)) {
                    inmap.insert(e.to_port, v.clone());
                }
            }
        }
        for ((n, p), v) in injected {
            if *n == nid {
                inmap.entry(p).or_insert_with(|| v.clone());
            }
        }
        for spec in nodes[nid].inputs() {
            if !inmap.contains_key(spec.name) {
                if let Some(d) = spec.default {
                    inmap.insert(spec.name, d);
                }
            }
        }

        let inputs = Inputs { map: inmap };
        let mut outputs = Outputs::default();
        let local: &mut dyn Any = &mut *locals[nid];
        let t0 = Instant::now();
        nodes[nid].process(ctx, local, &inputs, &mut outputs);
        timings.push((nid, t0.elapsed()));

        for (port, val) in outputs.map {
            store.insert((nid, port), val);
        }
    }

    store
}

/// Topologically sort `num_nodes` by `deps` (`(from, to)` pairs), Kahn. Reused for the top
/// graph and group bodies (so it takes plain index pairs, not `Edge`).
pub(crate) fn topo_order(
    num_nodes: usize,
    deps: &[(usize, usize)],
) -> Result<Vec<usize>, CompileError> {
    let mut indeg = vec![0usize; num_nodes];
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); num_nodes];
    for &(from, to) in deps {
        adj[from].push(to);
        indeg[to] += 1;
    }
    let mut order = Vec::with_capacity(num_nodes);
    let mut queue: Vec<usize> = (0..num_nodes).filter(|&i| indeg[i] == 0).collect();
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
    if order.len() != num_nodes {
        return Err(CompileError::Cycle);
    }
    Ok(order)
}

pub struct Mira {
    order: Vec<usize>,
    locals: Vec<Box<dyn Any + Send>>, // one per node instance (indexed by NodeId)
    ctx: Context,
    profile: Profile,
}

#[derive(Debug)]
pub enum CompileError {
    /// The graph has a dataflow cycle (cycles must route through a portal, which is acyclic for
    /// scheduling).
    Cycle,
}

impl Mira {
    /// Compile the graph: topo-sort, give each node a chance to `prepare` (with the registry in
    /// scope — e.g. `Map` builds its group body's sub-plan here), and allocate local state.
    pub fn compile(graph: &Graph) -> Result<Self, CompileError> {
        let deps: Vec<(usize, usize)> = graph
            .edges
            .iter()
            .map(|e| (e.from_node, e.to_node))
            .collect();
        let order = topo_order(graph.nodes.len(), &deps)?;
        for node in &graph.nodes {
            node.prepare(&graph.registry);
        }
        let locals = graph.nodes.iter().map(|node| node.new_local()).collect();
        Ok(Self {
            order,
            locals,
            ctx: Context::new(),
            profile: Profile::with_len(graph.nodes.len()),
        })
    }

    /// Access the shared context to install resources before running (e.g. calibration tables).
    pub fn context_mut(&mut self) -> &mut Context {
        &mut self.ctx
    }

    /// The always-on per-node latency profile, accumulated across ticks.
    pub fn profile(&self) -> &Profile {
        &self.profile
    }

    /// Run one tick (one frame); returns its wall-clock latency.
    pub fn run_tick(&mut self, graph: &Graph) -> Tick {
        let start = Instant::now();
        let Mira {
            order,
            locals,
            ctx,
            profile,
        } = self;
        ctx.advance();

        let injected: HashMap<(usize, &'static str), Value> = HashMap::new();
        let mut timings: Vec<(usize, Duration)> = Vec::new();
        let store = run_order(
            &graph.nodes,
            &graph.edges,
            order,
            locals,
            ctx,
            &injected,
            &mut timings,
        );
        for (nid, dur) in &timings {
            profile.record(*nid, *dur);
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
    store: Store,
}

impl Tick {
    pub fn output(&self, node: NodeId, port: &'static str) -> Option<&Value> {
        self.store.get(&(node.0, port))
    }
}
