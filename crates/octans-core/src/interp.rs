//! `Mira` — the interpreter engine (the live/edit tier; named for the variable star, since it
//! runs a graph that's constantly changing under edits). Compiles a topological order once,
//! owns each node's per-instance local state, and runs timed ticks.
//!
//! The top-level graph runs **level-parallel** ([`run_levels`]): the topo order is grouped into
//! depth-levels (antichains), and each level's mutually-independent nodes run concurrently via
//! rayon — disjoint `&mut` per-node state, shared `&` reads from the frozen store. This is the
//! task-parallel complement to `Map`'s data-parallelism (and the start of the `Vega` engine).
//! Group bodies inside `Map` use the sequential [`run_order`] (they're small and already inside
//! a parallel lane).

use crate::context::Context;
use crate::graph::{Edge, Graph, NodeId};
use crate::node::{Inputs, Node, Outputs};
use crate::profile::Profile;
use crate::strategy::StrategyHandle;
use crate::value::Value;
use rayon::prelude::*;
use std::any::Any;
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// A keyed map of values produced this tick: `(node index, output port) -> value`.
pub(crate) type Store = HashMap<(usize, &'static str), Value>;

type Injected = HashMap<(usize, &'static str), Value>;

/// The `(port, value)` outputs a single node produced.
type NodeOutputs = Vec<(&'static str, Value)>;

/// Evaluate one node: gather its inputs (injected boundary feeds, then connected edges, then
/// unconnected ports' defaults), run it, and return its outputs. Reads `store` immutably, so it
/// is safe to call concurrently for independent nodes in a level.
fn eval_node(
    nid: usize,
    nodes: &[Box<dyn Node>],
    edges: &[Edge],
    store: &Store,
    ctx: &Context,
    local: &mut dyn Any,
    injected: &Injected,
) -> NodeOutputs {
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
    nodes[nid].process(ctx, local, &inputs, &mut outputs);
    outputs.map.into_iter().collect()
}

/// Run `order` sequentially, returning every value produced. Used for `Map` group bodies.
pub(crate) fn run_order(
    nodes: &[Box<dyn Node>],
    edges: &[Edge],
    order: &[usize],
    locals: &mut [Box<dyn Any + Send>],
    ctx: &Context,
    injected: &Injected,
    timings: &mut Vec<(usize, Duration)>,
) -> Store {
    timings.clear();
    let mut store: Store = HashMap::new();
    for &nid in order {
        let local: &mut dyn Any = &mut *locals[nid];
        let t0 = Instant::now();
        let outs = eval_node(nid, nodes, edges, &store, ctx, local, injected);
        timings.push((nid, t0.elapsed()));
        for (port, val) in outs {
            store.insert((nid, port), val);
        }
    }
    store
}

/// Run the graph **level-parallel**: each depth-level's independent nodes run concurrently.
fn run_levels(
    nodes: &[Box<dyn Node>],
    edges: &[Edge],
    level_of: &[usize],
    max_level: usize,
    locals: &mut [Box<dyn Any + Send>],
    ctx: &Context,
    profile: &mut Profile,
) -> Store {
    let mut store: Store = HashMap::new();
    let empty: Injected = HashMap::new();

    for level in 0..=max_level {
        // All nodes at this level are mutually independent: run them concurrently, each with a
        // disjoint `&mut` to its own state, reading the frozen `store` by shared reference.
        let produced: Vec<(usize, NodeOutputs, Duration)> = locals
            .par_iter_mut()
            .enumerate()
            .filter(|(i, _)| level_of[*i] == level)
            .map(|(nid, local)| {
                let l: &mut dyn Any = &mut **local;
                let t0 = Instant::now();
                let outs = eval_node(nid, nodes, edges, &store, ctx, l, &empty);
                (nid, outs, t0.elapsed())
            })
            .collect();

        for (nid, outs, dur) in produced {
            for (port, val) in outs {
                store.insert((nid, port), val);
            }
            profile.record(nid, dur);
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
    level_of: Vec<usize>, // depth level per node (nodes at the same level run in parallel)
    max_level: usize,
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
    /// Compile the graph: topo-sort (cycle check), assign depth levels, let each node `prepare`
    /// (registry in scope — e.g. `Map` builds its group body's sub-plan), and allocate state.
    pub fn compile(graph: &Graph) -> Result<Self, CompileError> {
        let deps: Vec<(usize, usize)> = graph
            .edges
            .iter()
            .map(|e| (e.from_node, e.to_node))
            .collect();
        let order = topo_order(graph.nodes.len(), &deps)?;

        // Depth level = 1 + max predecessor level (computed in dependency order).
        let mut level_of = vec![0usize; graph.nodes.len()];
        for &nid in &order {
            let mut lvl = 0;
            for e in &graph.edges {
                if e.to_node == nid {
                    lvl = lvl.max(level_of[e.from_node] + 1);
                }
            }
            level_of[nid] = lvl;
        }
        let max_level = level_of.iter().copied().max().unwrap_or(0);

        for node in &graph.nodes {
            node.prepare(&graph.registry);
        }
        let locals = graph.nodes.iter().map(|node| node.new_local()).collect();
        Ok(Self {
            level_of,
            max_level,
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

    /// Run one tick (one frame), level-parallel; returns its wall-clock latency.
    pub fn run_tick(&mut self, graph: &Graph) -> Tick {
        let start = Instant::now();
        let Mira {
            level_of,
            max_level,
            locals,
            ctx,
            profile,
        } = self;
        ctx.advance();

        let store = run_levels(
            &graph.nodes,
            &graph.edges,
            level_of,
            *max_level,
            locals,
            ctx,
            profile,
        );

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

/// How long to benchmark each variant when autotuning.
pub struct TuneConfig {
    pub warmup: usize,
    pub trials: usize,
}

impl Default for TuneConfig {
    fn default() -> Self {
        Self {
            warmup: 2,
            trials: 5,
        }
    }
}

/// The outcome of tuning one [`Strategy`](crate::strategy::Strategy) node.
pub struct TuneResult {
    pub node: NodeId,
    pub chosen: usize,
    pub chosen_name: &'static str,
    /// Best (min) observed latency per variant, in declaration order.
    pub per_variant_best: Vec<Duration>,
}

impl Mira {
    /// Autotune: for each strategy node, benchmark every variant on the live graph (the
    /// profiler measures the node's latency; we take the min over trials to shrug off noise) and
    /// select the fastest. Greedy per strategy.
    ///
    /// v1 is **speed-only**: variants are assumed output-equivalent (author-asserted). Verify-by-
    /// default (differential-test the variants' outputs) is the planned next step — it needs a
    /// registered per-type comparator.
    pub fn tune(
        &mut self,
        graph: &Graph,
        strategies: &[(NodeId, StrategyHandle)],
        cfg: TuneConfig,
    ) -> Vec<TuneResult> {
        let mut results = Vec::new();
        for (sid, handle) in strategies {
            let count = handle.variant_count();
            let mut best = vec![Duration::MAX; count];
            for (v, slot) in best.iter_mut().enumerate() {
                handle.select(v);
                for _ in 0..cfg.warmup {
                    self.run_tick(graph);
                }
                for _ in 0..cfg.trials {
                    self.run_tick(graph);
                    let t = self.profile().node(*sid).last;
                    if t < *slot {
                        *slot = t;
                    }
                }
            }
            let chosen = best
                .iter()
                .enumerate()
                .min_by_key(|(_, d)| **d)
                .map(|(v, _)| v)
                .unwrap_or(0);
            handle.select(chosen);
            results.push(TuneResult {
                node: *sid,
                chosen,
                chosen_name: handle.variant_name(chosen),
                per_variant_best: best,
            });
        }
        results
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
