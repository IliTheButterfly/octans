# Octans

**A realtime, node-based processing engine that compiles itself away.**

Author a processing pipeline as a graph of typed nodes, edit it live, and run it in parallel —
then (eventually) compile it into a self-optimizing component you embed in a larger program.
Octans is built in Rust, around an open type system and a hard commitment to *zero-copy,
parallel, measurable* execution.

> Octans is the successor to three abandoned attempts (Python, C++, Rust) at a multi-camera 3D
> tracker. Somewhere along the way the *engine* became the point and the tracker became a test
> fixture. The name is the southern constellation of the navigator's octant; its engines are
> named after stars.

> ⚠️ **Status: early, fast-moving, experimental.** The engine core works and is tested end to
> end (it even triangulates 3D points), but APIs change freely and there is no editor or GPU
> backend yet. See [Status](#status).

## What it looks like

```rust
use octans_core::*;
use octans_nodes::*;

let mut reg = Registry::new();
register_primitives(&mut reg);
register_node_types(&mut reg);

// A reusable detection pipeline as a group…
let detect = group("detect", |g| {
    let t = g.add(Threshold);   // thr is a parameter port (default 128)
    let b = g.add(BlobCount);
    g.connect(t, "mask", b, "mask");
    g.input("image", t, "image");
    g.output("count", b, "count");
});

let mut graph = Graph::new(reg);
let cam = graph.add(SyntheticCamera { w: 128, h: 128, blobs: vec![(30,30,8),(90,40,10),(60,100,6)] });
let m   = graph.add(Map::group(&detect));   // …fanned over every frame, in parallel
graph.connect(cam, "frames", m, "image").unwrap();

let mut engine = Mira::compile(&graph).unwrap();   // type-checked, topo-sorted, level-parallel
let tick = engine.run_tick(&graph);
```

Connections are **type-checked when you make them**; the graph runs **level-parallel** (and
`Map` fans data-parallel over lanes); every node's latency is **profiled every tick**; and a
`Strategy` node's interchangeable variants can be **autotuned** — benchmarked and verified, then
the fastest is selected.

## Concepts

- **Open type system.** Types are registered (built-in or from a plugin) under stable, named
  ids — not `std::any::TypeId`. Values flow type-erased and zero-copy (`Arc`); ports carry a
  `TypeSpec`; connections are validated against the registry.
- **Typed authoring.** Write a node as a plain Rust `fn` with `#[node]`; the macro derives the
  ports and the type-erase glue. Inputs can be data, `#[param(default = …)]` knobs, `#[local]`
  per-instance state, or the shared `#[ctx]` context.
- **State, by exclusivity.** `&self` is immutable shared logic; `&mut Local` is per-instance
  state the runtime owns (replicated per lane on fan-out); `&Context` is shared read-mostly
  globals. Whatever is mutable is exclusively owned or explicitly synchronized — never an
  implicit data race. (This is why parallel execution simply *compiles*.)
- **Composition.** `group`s flatten into the runtime plan (a boundary port is a compile-time
  edge-splice, not a runtime node). `Map` fans a node-or-group over N lanes (zipping K inputs to
  M outputs). `Gather`/`Scatter` pack and unpack vectors. `Strategy` holds interchangeable
  variants behind one signature.
- **Feedback.** `Portal`s are temporal (z⁻¹) feedback edges: a loop in intent, a DAG in
  dataflow, so it still schedules. Self-correcting control loops are authored in the graph.
- **Self-optimization.** An always-on profiler measures every node; `Mira::tune` benchmarks a
  `Strategy`'s variants, **verifies** they agree with the reference, and selects the fastest.

## Architecture

| Crate | Role |
|---|---|
| `octans-core` | the engine: type registry, graph, `Mira` interpreter + scheduler, `Map`/`Gather`/`Scatter`/`Strategy`, groups, portals, profiler, autotuner |
| `octans-macros` | the `#[node]` authoring attribute |
| `octans-nodes` | standard nodes (image ops, the tracking/triangulation domain) — dogfoods the macro across a crate boundary |

**Engine tiers** (named after stars): **Mira** — the interpreter (live/edit). **Vega** — a
JIT tier (planned). **Canopus** — the AOT/codegen release tier (planned). The autotuner is
**Pyxis** (the compass).

## Build & test

```sh
cargo test --workspace          # full suite
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

Pure-Rust and dependency-light by design. CI runs fmt + clippy (deny-warnings) + tests on Linux
and Windows.

## Status

**Built & tested:** open type registry · `#[node]` authoring · connect-time type checking ·
`Mira` interpreter · level-parallel scheduler · `Map` (data-parallel fan-out, zip K×M) · groups
(flatten) · `Gather`/`Scatter` · portals · per-instance local + shared context state ·
always-on profiler · `Strategy` variant groups · autotuner (benchmark + verify + select) ·
N-view DLT triangulation.

**Not yet:** a node editor (GUI), a GPU (wgpu) backend, IR serialization / codegen, and the
`switch`/`loop` base components.

## Roadmap

- IR serialization (persist graphs) and codegen (the Vega/Canopus release path)
- `switch` (data routing) and `loop` (bounded iteration) base components
- A headless graph visualizer, then the egui node editor
- GPU (wgpu) backend → CPU-vs-GPU become autotuned `Strategy` variants
- The full multi-camera tracking pipeline, wired end-to-end

## License

MIT OR Apache-2.0.
