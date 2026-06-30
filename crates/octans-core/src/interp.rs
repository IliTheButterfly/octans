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
use std::panic::AssertUnwindSafe;
use std::time::{Duration, Instant};

/// A keyed map of values produced this tick: `(node index, output port) -> value`.
pub(crate) type Store = HashMap<(usize, &'static str), Value>;

type Injected = HashMap<(usize, &'static str), Value>;

/// The `(port, value)` outputs a single node produced.
type NodeOutputs = Vec<(&'static str, Value)>;

/// A node that failed during a tick — recorded on the [`Tick`] instead of crashing the engine.
#[derive(Debug, Clone)]
pub struct Fault {
    pub node: NodeId,
    pub message: String,
}

/// The outcome of evaluating one node in a tick.
enum Eval {
    /// Ran to completion; carries the `(port, value)` pairs it wrote.
    Produced(NodeOutputs),
    /// `process` panicked; carries the panic message. The engine keeps running.
    Faulted(String),
}

/// Best-effort extraction of a panic payload's message (`&str` / `String`, else a placeholder).
fn panic_message(payload: Box<dyn Any + Send>) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "node panicked (non-string payload)".to_string()
    }
}

/// Evaluate one node: gather its inputs (injected boundary feeds, then connected edges, then
/// unconnected ports' defaults), run it, and return its outputs. Reads `store` immutably, so it
/// is safe to call concurrently for independent nodes in a level.
///
/// `process` is run inside [`catch_unwind`](std::panic::catch_unwind): a panicking node yields
/// [`Eval::Faulted`] rather than unwinding through the parallel tick and aborting the engine.
fn eval_node(
    nid: usize,
    nodes: &[Box<dyn Node>],
    edges: &[Edge],
    store: &Store,
    ctx: &Context,
    local: &mut dyn Any,
    injected: &Injected,
) -> Eval {
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
    let node = &nodes[nid];
    // A panic here must not unwind across rayon's join boundary and abort the whole tick.
    let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
        let mut outputs = Outputs::default();
        node.process(ctx, local, &inputs, &mut outputs);
        outputs
    }));
    match result {
        Ok(outputs) => Eval::Produced(outputs.map.into_iter().collect()),
        Err(payload) => Eval::Faulted(panic_message(payload)),
    }
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
        let ev = eval_node(nid, nodes, edges, &store, ctx, local, injected);
        timings.push((nid, t0.elapsed()));
        // Inside a Map lane, a faulting body node degrades gracefully: it simply produces no
        // outputs (the top-level tick is what surfaces faults to the caller).
        if let Eval::Produced(outs) = ev {
            for (port, val) in outs {
                store.insert((nid, port), val);
            }
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
) -> (Store, Vec<Fault>) {
    let mut store: Store = HashMap::new();
    let mut faults: Vec<Fault> = Vec::new();
    let empty: Injected = HashMap::new();

    for level in 0..=max_level {
        // All nodes at this level are mutually independent: run them concurrently, each with a
        // disjoint `&mut` to its own state, reading the frozen `store` by shared reference.
        let produced: Vec<(usize, Eval, Duration)> = locals
            .par_iter_mut()
            .enumerate()
            .filter(|(i, _)| level_of[*i] == level)
            .map(|(nid, local)| {
                let l: &mut dyn Any = &mut **local;
                let t0 = Instant::now();
                let ev = eval_node(nid, nodes, edges, &store, ctx, l, &empty);
                (nid, ev, t0.elapsed())
            })
            .collect();

        for (nid, ev, dur) in produced {
            match ev {
                Eval::Produced(outs) => {
                    for (port, val) in outs {
                        store.insert((nid, port), val);
                    }
                }
                Eval::Faulted(message) => faults.push(Fault {
                    node: NodeId(nid),
                    message,
                }),
            }
            profile.record(nid, dur);
        }
    }
    (store, faults)
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

        let (store, faults) = run_levels(
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
            faults,
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
    /// Variants excluded because an output disagreed with the reference (variant 0).
    pub rejected: Vec<usize>,
}

impl Mira {
    /// Autotune: for each strategy node, benchmark every variant on the live graph (the
    /// profiler measures the node's latency; we take the min over trials to shrug off noise),
    /// **verify** each variant agrees with the reference (variant 0) on every output that has a
    /// registered comparator, and select the fastest *verified* variant. Greedy per strategy.
    ///
    /// Verify-by-default: a variant whose comparable outputs differ from the reference is
    /// rejected (it isn't actually equivalent). Outputs whose type has no comparator can't be
    /// checked, so they're trusted (the author asserted equivalence).
    pub fn tune(
        &mut self,
        graph: &Graph,
        strategies: &[(NodeId, StrategyHandle)],
        cfg: TuneConfig,
    ) -> Vec<TuneResult> {
        let mut results = Vec::new();
        for (sid, handle) in strategies {
            let count = handle.variant_count();
            let out_specs = graph.nodes[sid.0].outputs();
            let mut best = vec![Duration::MAX; count];
            // Captured outputs per variant (one Option<Value> per output port).
            let mut sample: Vec<Vec<Option<Value>>> = vec![Vec::new(); count];

            for (v, slot) in best.iter_mut().enumerate() {
                handle.select(v);
                for _ in 0..cfg.warmup {
                    self.run_tick(graph);
                }
                for _ in 0..cfg.trials {
                    let tick = self.run_tick(graph);
                    let t = self.profile().node(*sid).last;
                    if t < *slot {
                        *slot = t;
                    }
                    sample[v] = out_specs
                        .iter()
                        .map(|p| tick.output(*sid, p.name).cloned())
                        .collect();
                }
            }

            // Verify each variant against the reference (variant 0).
            let mut rejected: Vec<usize> = Vec::new();
            if count > 1 && !sample[0].is_empty() {
                for (v, sv) in sample.iter().enumerate().skip(1) {
                    let differs = out_specs.iter().enumerate().any(|(k, p)| {
                        match (graph.registry.comparator(p.ty.id), &sample[0][k], &sv[k]) {
                            (Some(cmp), Some(a), Some(b)) => !cmp(a, b),
                            _ => false,
                        }
                    });
                    if differs {
                        rejected.push(v);
                    }
                }
            }

            let chosen = (0..count)
                .filter(|v| !rejected.contains(v))
                .min_by_key(|&v| best[v])
                .unwrap_or(0);
            handle.select(chosen);
            results.push(TuneResult {
                node: *sid,
                chosen,
                chosen_name: handle.variant_name(chosen),
                per_variant_best: best,
                rejected,
            });
        }
        results
    }
}

/// The result of one tick: latency, the values each node produced (for inspection/sinks), and
/// any faults — nodes whose `process` panicked. A non-empty `faults` means the tick still
/// completed (the engine isolated those nodes); inspect it instead of relying on a clean run.
pub struct Tick {
    pub latency: Duration,
    store: Store,
    pub faults: Vec<Fault>,
}

impl Tick {
    pub fn output(&self, node: NodeId, port: &'static str) -> Option<&Value> {
        self.store.get(&(node.0, port))
    }

    /// True if every node ran without panicking this tick.
    pub fn ok(&self) -> bool {
        self.faults.is_empty()
    }
}
