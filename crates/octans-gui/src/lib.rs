//! Octans GUI — a read-only "watch it run" visualizer for an Octans graph.
//!
//! All logic lives in the library (and sibling modules) so it compiles and unit-tests headlessly.
//! The only window-opening call (`eframe::run_native`) lives in `src/main.rs`, which `cargo
//! test`/`clippy` compile but never execute — so CI never needs a display.

pub mod canvas;
pub mod history;
pub mod inspector;
pub mod layout;
pub mod log_panel;
pub mod model;
pub mod palette;
pub mod params;
pub mod profiler;
pub mod pyxis;
pub mod scene;
pub mod schedule;

use eframe::egui;
use octans_core::{
    Catalog, Diagnostic, Fault, Graph, GraphSpec, Mira, NodeId, NodeRegistry, Registry,
    StrategyHandle, Tick, TuneResult, Value,
};
use scene::SceneKind;
use std::collections::{HashMap, HashSet, VecDeque};
use std::time::Duration;

/// Format a duration compactly (ms above 1ms, else µs). Shared by the profiler/schedule panels.
pub(crate) fn fmt_dur(d: Duration) -> String {
    let us = d.as_secs_f64() * 1e6;
    if us >= 1000.0 {
        format!("{:.2} ms", us / 1000.0)
    } else {
        format!("{us:.1} µs")
    }
}

/// Run-control mode.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RunState {
    Stopped,
    Stepping,
    Playing,
}

/// How many ticks to run this frame. Pure (no egui) → unit-tested. `Stepping` yields exactly one
/// (the caller then flips back to `Stopped`); `Playing` accumulates wall-time at `tick_hz`.
pub fn ticks_to_run(run: RunState, tick_hz: f32, dt: f32, acc: &mut f32) -> u32 {
    match run {
        RunState::Stopped => 0,
        RunState::Stepping => 1,
        RunState::Playing => {
            if tick_hz <= 0.0 {
                return 0;
            }
            let period = 1.0 / tick_hz;
            *acc += dt;
            let mut n = 0;
            while *acc >= period {
                *acc -= period;
                n += 1;
                if n >= 16 {
                    *acc = 0.0; // don't spiral if the frame stalled
                    break;
                }
            }
            n
        }
    }
}

/// What the panels need from the most recent tick (`Tick` isn't `Clone` and borrows the engine).
#[derive(Default, Clone)]
pub struct TickSnapshot {
    pub latency: Duration,
    pub faulted: HashSet<NodeId>,
    pub skipped: HashSet<NodeId>,
    pub fault_msgs: Vec<Fault>,
}

/// A capped ring of diagnostics accumulated across ticks.
pub struct LogRing {
    buf: VecDeque<Diagnostic>,
    cap: usize,
}

impl LogRing {
    pub fn new(cap: usize) -> Self {
        Self {
            buf: VecDeque::new(),
            cap,
        }
    }
    pub fn extend(&mut self, ds: impl IntoIterator<Item = Diagnostic>) {
        for d in ds {
            if self.buf.len() == self.cap {
                self.buf.pop_front();
            }
            self.buf.push_back(d);
        }
    }
    pub fn clear(&mut self) {
        self.buf.clear();
    }
    pub fn iter(&self) -> impl Iterator<Item = &Diagnostic> {
        self.buf.iter()
    }
    pub fn len(&self) -> usize {
        self.buf.len()
    }
    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }
}

/// Pan/zoom transform for the canvas.
#[derive(Clone, Copy)]
pub struct Camera {
    pub pan: egui::Vec2,
    pub zoom: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            pan: egui::vec2(40.0, 40.0),
            zoom: 1.0,
        }
    }
}

impl Camera {
    /// World → screen, given the canvas's screen-space origin (its top-left).
    pub fn to_screen(&self, world: egui::Pos2, origin: egui::Pos2) -> egui::Pos2 {
        origin + (world.to_vec2() * self.zoom + self.pan)
    }
    pub fn to_screen_rect(&self, world: egui::Rect, origin: egui::Pos2) -> egui::Rect {
        egui::Rect::from_min_max(
            self.to_screen(world.min, origin),
            self.to_screen(world.max, origin),
        )
    }
}

/// Profiler table sort key.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ProfileKey {
    Id,
    Type,
    Last,
    Mean,
    Max,
    Samples,
}

#[derive(Clone, Copy)]
pub struct ProfileSort {
    pub key: ProfileKey,
    pub desc: bool,
}

impl Default for ProfileSort {
    fn default() -> Self {
        Self {
            key: ProfileKey::Mean,
            desc: true,
        }
    }
}

/// The application: owns the live graph + engine and the read-only view/run state.
pub struct OctansApp {
    pub(crate) graph: Graph,
    /// `None` when the current (edited) graph doesn't compile — see `compile_error`.
    pub(crate) engine: Option<Mira>,
    pub(crate) compile_error: Option<String>,
    pub(crate) view: model::ViewGraph,
    pub(crate) layout: layout::Layout,

    pub(crate) scene_kind: SceneKind,
    pub(crate) run: RunState,
    pub(crate) tick_hz: f32,
    pub(crate) accumulator: f32,
    pub(crate) tick_count: u64,
    pub(crate) last_tick: Option<TickSnapshot>,

    pub(crate) log: LogRing,
    pub(crate) camera: Camera,
    pub(crate) follow_log: bool,
    pub(crate) show_latency_overlay: bool,
    pub(crate) show_critical_path: bool,
    pub(crate) profiler_sort: ProfileSort,

    // data viewers (feature 3)
    pub(crate) selected: Option<NodeId>,
    /// Latest tick's outputs, keyed by (node, port). `Value` is `Arc`-backed → cheap to clone.
    pub(crate) values: HashMap<(usize, &'static str), Value>,
    /// Recent numeric components per output port, for sparklines.
    pub(crate) history: HashMap<(usize, &'static str), VecDeque<Vec<f64>>>,
    /// Cached image textures, with the tick they were uploaded on.
    pub(crate) textures: HashMap<(usize, &'static str), (u64, egui::TextureHandle)>,

    // autotuner / strategies (feature 4)
    pub(crate) strategies: Vec<(NodeId, StrategyHandle)>,
    pub(crate) tune_results: HashMap<usize, TuneResult>,
    pub(crate) tune_warmup: usize,
    pub(crate) tune_trials: usize,

    // node catalog / palette (editor groundwork)
    pub(crate) catalog: Catalog,
    pub(crate) show_palette: bool,
    /// Transient message from the last rejected edit (e.g. a type-mismatched wire).
    pub(crate) edit_error: Option<String>,
    /// Manual node positions (world top-left), overriding auto-layout where set. Keyed by NodeId.0.
    pub(crate) manual_pos: HashMap<usize, egui::Pos2>,
    /// Serde factories, for rebuilding a node from edited config (param editing / load).
    pub(crate) node_registry: NodeRegistry,
    /// The config being edited in the inspector: `(node, json)`. Reloaded when selection changes.
    pub(crate) param_edit: Option<(usize, serde_json::Value)>,

    // save / load
    pub(crate) graph_path: String,
    pub(crate) io_status: Option<String>,

    // undo / redo
    pub(crate) undo_stack: Vec<history::EditAction>,
    pub(crate) redo_stack: Vec<history::EditAction>,
    /// A node drag in progress: `(node, its manual position before the drag)`.
    pub(crate) drag_start: Option<(usize, Option<egui::Pos2>)>,
}

impl OctansApp {
    pub fn new(_cc: &eframe::CreationContext<'_>, kind: SceneKind) -> Self {
        Self::from_scene(kind)
    }

    /// Construct without an eframe context (everything here is plain data) — used by `new` and by
    /// headless tests.
    pub fn from_scene(kind: SceneKind) -> Self {
        let scene = kind.build();
        let view = model::ViewGraph::from_graph(&scene.graph);
        let layout = layout::layout(&view);
        let mut catalog = Catalog::new();
        octans_nodes::register_std_catalog(&mut catalog);
        let mut node_registry = NodeRegistry::new();
        octans_nodes::register_std_factories(&mut node_registry);
        Self {
            graph: scene.graph,
            engine: Some(scene.engine),
            compile_error: None,
            view,
            layout,
            scene_kind: kind,
            run: RunState::Stopped,
            tick_hz: 10.0,
            accumulator: 0.0,
            tick_count: 0,
            last_tick: None,
            log: LogRing::new(2000),
            camera: Camera::default(),
            follow_log: true,
            show_latency_overlay: false,
            show_critical_path: false,
            profiler_sort: ProfileSort::default(),
            selected: None,
            values: HashMap::new(),
            history: HashMap::new(),
            textures: HashMap::new(),
            strategies: scene.strategies,
            tune_results: HashMap::new(),
            tune_warmup: 2,
            tune_trials: 5,
            catalog,
            show_palette: false,
            edit_error: None,
            manual_pos: HashMap::new(),
            node_registry,
            param_edit: None,
            graph_path: "octans_graph.json".to_string(),
            io_status: None,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            drag_start: None,
        }
    }

    /// Save the graph (as a `GraphSpec`) plus manual node positions to `graph_path`. Node types
    /// without a serde factory serialize with a null config and won't load back — the load side
    /// reports that precisely.
    pub(crate) fn save_graph(&mut self) {
        let layout: Vec<(usize, f32, f32)> = self
            .manual_pos
            .iter()
            .map(|(i, p)| (*i, p.x, p.y))
            .collect();
        let doc = serde_json::json!({ "graph": self.graph.to_spec(), "layout": layout });
        let res = serde_json::to_string_pretty(&doc)
            .map_err(|e| e.to_string())
            .and_then(|s| std::fs::write(&self.graph_path, s).map_err(|e| e.to_string()));
        self.io_status = Some(match res {
            Ok(()) => format!("saved → {}", self.graph_path),
            Err(e) => format!("save failed: {e}"),
        });
    }

    /// Load a graph + layout from `graph_path`, rebuilding via the serde factories.
    pub(crate) fn load_graph(&mut self) {
        let doc: serde_json::Value = match std::fs::read_to_string(&self.graph_path)
            .map_err(|e| e.to_string())
            .and_then(|t| serde_json::from_str(&t).map_err(|e| e.to_string()))
        {
            Ok(v) => v,
            Err(e) => {
                self.io_status = Some(format!("load failed: {e}"));
                return;
            }
        };
        let spec: GraphSpec =
            match serde_json::from_value(doc.get("graph").cloned().unwrap_or_default()) {
                Ok(s) => s,
                Err(e) => {
                    self.io_status = Some(format!("bad graph: {e}"));
                    return;
                }
            };
        let mut reg = Registry::new();
        octans_core::register_primitives(&mut reg);
        octans_nodes::register_node_types(&mut reg);
        octans_nodes::register_tracking_types(&mut reg);
        match spec.build(reg, &self.node_registry) {
            Ok(graph) => {
                self.graph = graph;
                self.strategies.clear();
                self.tune_results.clear();
                self.selected = None;
                self.manual_pos.clear();
                if let Some(arr) = doc.get("layout").and_then(|l| l.as_array()) {
                    for e in arr {
                        if let Some(a) = e.as_array() {
                            if let (Some(i), Some(x), Some(y)) = (
                                a.first().and_then(|v| v.as_u64()),
                                a.get(1).and_then(|v| v.as_f64()),
                                a.get(2).and_then(|v| v.as_f64()),
                            ) {
                                self.manual_pos
                                    .insert(i as usize, egui::pos2(x as f32, y as f32));
                            }
                        }
                    }
                }
                self.rebuild_after_edit();
                self.undo_stack.clear();
                self.redo_stack.clear();
                self.io_status = Some(format!("loaded ← {}", self.graph_path));
            }
            Err(e) => self.io_status = Some(format!("load failed: {e:?}")),
        }
    }

    /// Recompile the engine after a change that doesn't alter the graph's *shape* (e.g. a param
    /// edit — ports/edges unchanged), preserving view/layout/selection and the tick counter.
    pub(crate) fn recompile(&mut self) {
        match Mira::compile(&self.graph) {
            Ok(m) => {
                self.engine = Some(m);
                self.compile_error = None;
            }
            Err(e) => {
                self.engine = None;
                self.compile_error = Some(format!("{e:?}"));
            }
        }
    }

    /// Recompute auto-layout, then apply any manual node positions on top (so dragged nodes stay
    /// put across edits while new/untouched nodes auto-place).
    pub(crate) fn relayout(&mut self) {
        self.layout = layout::layout(&self.view);
        for (id, pos) in &self.manual_pos {
            if let Some(r) = self.layout.rects.get_mut(*id) {
                *r = egui::Rect::from_min_size(*pos, r.size());
            }
        }
    }

    /// Remove a node (tombstone — keeps other NodeIds stable), recording the edit, and recompile.
    pub(crate) fn delete_node(&mut self, id: NodeId) {
        let Some((type_id, config)) = self
            .graph
            .node(id)
            .map(|n| (n.node_type().to_string(), n.to_json()))
        else {
            return;
        };
        let edges: Vec<history::EdgeRec> = self
            .graph
            .edges()
            .filter(|e| e.from == id || e.to == id)
            .map(|e| {
                (
                    e.from.0,
                    e.from_port.to_string(),
                    e.to.0,
                    e.to_port.to_string(),
                )
            })
            .collect();
        let pos = self.manual_pos.get(&id.0).map(|p| (p.x, p.y));
        self.push_edit(history::EditAction::DeleteNode {
            id: id.0,
            type_id,
            config,
            edges,
            pos,
        });

        self.graph.remove_node(id);
        self.manual_pos.remove(&id.0);
        if self.selected == Some(id) {
            self.selected = None;
        }
        self.rebuild_after_edit();
    }

    /// Connect two ports (replacing any existing wire into the target), validating first so a
    /// rejected wire doesn't drop the old one. Recompiles on success; records `edit_error` on
    /// failure. Returns whether the graph changed.
    pub(crate) fn try_connect(
        &mut self,
        from: NodeId,
        from_port: &str,
        to: NodeId,
        to_port: &str,
    ) -> bool {
        match self.graph.can_connect(from, from_port, to, to_port) {
            Ok(()) => {
                let replaced: Vec<(usize, String)> = self
                    .graph
                    .edges()
                    .filter(|e| e.to == to && e.to_port == to_port)
                    .map(|e| (e.from.0, e.from_port.to_string()))
                    .collect();
                self.graph.disconnect_input(to, to_port);
                let _ = self.graph.connect(from, from_port, to, to_port);
                self.push_edit(history::EditAction::Connect {
                    from: from.0,
                    from_port: from_port.to_string(),
                    to: to.0,
                    to_port: to_port.to_string(),
                    replaced,
                });
                self.edit_error = None;
                self.rebuild_after_edit();
                true
            }
            Err(e) => {
                self.edit_error = Some(format!("{e:?}"));
                false
            }
        }
    }

    /// Per-node last-tick latency, index-aligned with `NodeId.0` (for the schedule/critical-path).
    /// All zero when the graph doesn't currently compile.
    pub(crate) fn latencies(&self) -> Vec<Duration> {
        match &self.engine {
            Some(e) => (0..self.view.nodes.len())
                .map(|i| e.profile().node(NodeId(i)).last)
                .collect(),
            None => vec![Duration::ZERO; self.view.nodes.len()],
        }
    }

    /// Re-derive the view/layout from the (edited) graph and recompile the engine. The graph stays
    /// editable even when invalid: on a compile error we drop the engine and surface the message,
    /// rather than refusing the edit. Resets per-tick display state (a new engine has no history).
    pub(crate) fn rebuild_after_edit(&mut self) {
        self.view = model::ViewGraph::from_graph(&self.graph);
        self.relayout();
        self.recompile();
        self.param_edit = None;
        self.run = RunState::Stopped;
        self.accumulator = 0.0;
        self.tick_count = 0;
        self.last_tick = None;
        self.values.clear();
        self.history.clear();
        self.textures.clear();
        self.tune_results.clear();
        if self.selected.map(|n| n.0 >= self.graph.node_count()) == Some(true) {
            self.selected = None;
        }
    }

    /// Rebuild everything for a fresh scene.
    pub fn set_scene(&mut self, kind: SceneKind) {
        let scene = kind.build();
        self.view = model::ViewGraph::from_graph(&scene.graph);
        self.manual_pos.clear();
        self.relayout();
        self.graph = scene.graph;
        self.engine = Some(scene.engine);
        self.compile_error = None;
        self.strategies = scene.strategies;
        self.scene_kind = kind;
        self.run = RunState::Stopped;
        self.accumulator = 0.0;
        self.tick_count = 0;
        self.last_tick = None;
        self.log.clear();
        self.camera = Camera::default();
        self.selected = None;
        self.values.clear();
        self.history.clear();
        self.textures.clear();
        self.tune_results.clear();
        self.param_edit = None;
        self.undo_stack.clear();
        self.redo_stack.clear();
    }

    /// Fold one tick's results into the panels' state.
    pub fn ingest_tick(&mut self, tick: Tick) {
        self.log.extend(tick.diagnostics.iter().cloned());

        // Capture this tick's outputs (Arc clones — cheap) and append numeric history.
        self.values.clear();
        for (id, port, val) in tick.outputs() {
            self.values.insert((id.0, port), val.clone());
            if let Some(comps) = inspector::scalar_components(val) {
                let h = self.history.entry((id.0, port)).or_default();
                h.push_back(comps);
                while h.len() > 120 {
                    h.pop_front();
                }
            }
        }

        let faulted = tick.faults.iter().map(|f| f.node).collect();
        let skipped = tick.skipped.iter().copied().collect();
        self.last_tick = Some(TickSnapshot {
            latency: tick.latency,
            faulted,
            skipped,
            fault_msgs: tick.faults.clone(),
        });
        self.tick_count += 1;
    }

    fn toolbar_ui(&mut self, ui: &mut egui::Ui) {
        let mut do_save = false;
        let mut do_load = false;
        ui.horizontal(|ui| {
            ui.menu_button("File", |ui| {
                ui.horizontal(|ui| {
                    ui.label("path");
                    ui.text_edit_singleline(&mut self.graph_path);
                });
                if ui.button("💾 Save").clicked() {
                    do_save = true;
                    ui.close_menu();
                }
                if ui.button("📂 Load").clicked() {
                    do_load = true;
                    ui.close_menu();
                }
            });
            ui.separator();
            if ui
                .add_enabled(!self.undo_stack.is_empty(), egui::Button::new("↩"))
                .on_hover_text("undo (Ctrl+Z)")
                .clicked()
            {
                self.undo();
            }
            if ui
                .add_enabled(!self.redo_stack.is_empty(), egui::Button::new("↪"))
                .on_hover_text("redo (Ctrl+Shift+Z)")
                .clicked()
            {
                self.redo();
            }
            ui.separator();
            if ui
                .selectable_label(self.run == RunState::Playing, "▶ Play")
                .clicked()
            {
                self.run = RunState::Playing;
            }
            if ui.button("⏭ Step").clicked() {
                self.run = RunState::Stepping;
            }
            if ui.button("⏹ Stop").clicked() {
                self.run = RunState::Stopped;
            }
            ui.separator();
            ui.add(egui::Slider::new(&mut self.tick_hz, 0.5..=120.0).text("Hz"));
            ui.separator();
            ui.label(format!("tick {}", self.tick_count));
            if let Some(t) = &self.last_tick {
                ui.label(format!("· {:.2} ms", t.latency.as_secs_f64() * 1e3));
                if !t.faulted.is_empty() {
                    ui.colored_label(
                        egui::Color32::from_rgb(240, 90, 90),
                        format!("⚠ {} faulted", t.faulted.len()),
                    );
                }
                if !t.skipped.is_empty() {
                    ui.colored_label(
                        egui::Color32::from_rgb(200, 200, 120),
                        format!("⤳ {} skipped", t.skipped.len()),
                    );
                }
            }
            ui.separator();

            let mut kind = self.scene_kind;
            egui::ComboBox::from_label("scene")
                .selected_text(kind.label())
                .show_ui(ui, |ui| {
                    for k in SceneKind::ALL {
                        ui.selectable_value(&mut kind, k, k.label());
                    }
                });
            if kind != self.scene_kind {
                self.set_scene(kind);
            }

            ui.separator();
            ui.checkbox(&mut self.show_latency_overlay, "latency overlay");
            ui.toggle_value(&mut self.show_palette, "🎨 palette");
            if let Some(err) = &self.compile_error {
                ui.separator();
                ui.colored_label(
                    egui::Color32::from_rgb(240, 90, 90),
                    format!("⚠ won't compile: {err}"),
                );
            }
            if let Some(err) = &self.edit_error {
                ui.separator();
                ui.colored_label(egui::Color32::from_rgb(230, 170, 70), format!("✗ {err}"));
            }
            if let Some(status) = &self.io_status {
                ui.separator();
                ui.weak(status);
            }
        });
        if do_save {
            self.save_graph();
        }
        if do_load {
            self.load_graph();
        }
    }
}

impl eframe::App for OctansApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Advance the engine.
        let dt = ctx.input(|i| i.stable_dt);
        let n = ticks_to_run(self.run, self.tick_hz, dt, &mut self.accumulator);
        for _ in 0..n {
            let tick = {
                let Some(engine) = self.engine.as_mut() else {
                    self.run = RunState::Stopped;
                    break;
                };
                engine.run_tick(&self.graph)
            };
            self.ingest_tick(tick);
        }
        if self.run == RunState::Stepping {
            self.run = RunState::Stopped;
        }

        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| self.toolbar_ui(ui));
        egui::TopBottomPanel::bottom("log")
            .resizable(true)
            .default_height(160.0)
            .show(ctx, |ui| self.log_ui(ui));
        egui::SidePanel::right("profiler")
            .resizable(true)
            .default_width(360.0)
            .show(ctx, |ui| self.profiler_ui(ui));
        egui::SidePanel::left("schedule")
            .resizable(true)
            .default_width(240.0)
            .show(ctx, |ui| {
                self.autotuner_ui(ui);
                self.schedule_ui(ui);
            });
        egui::CentralPanel::default().show(ctx, |ui| self.canvas_ui(ui));
        self.inspector_window(ctx);
        self.palette_window(ctx);

        // Delete the selected node with the Delete key (unless a text field has focus).
        if let Some(sel) = self.selected {
            if ctx.input(|i| i.key_pressed(egui::Key::Delete)) && !ctx.wants_keyboard_input() {
                self.delete_node(sel);
            }
        }

        // Undo / redo shortcuts (Ctrl+Z / Ctrl+Shift+Z or Ctrl+Y).
        if !ctx.wants_keyboard_input() {
            let (undo_press, redo_press) = ctx.input(|i| {
                let z = i.key_pressed(egui::Key::Z);
                let y = i.key_pressed(egui::Key::Y);
                let cmd = i.modifiers.command;
                (
                    cmd && z && !i.modifiers.shift,
                    cmd && (y || (z && i.modifiers.shift)),
                )
            });
            if undo_press {
                self.undo();
            } else if redo_press {
                self.redo();
            }
        }

        // While playing, schedule the next repaint at the tick rate — *not* every monitor frame —
        // so we don't spin the CPU. When stopped/stepping we request nothing and egui idles
        // (reactive: it only repaints on input).
        if self.run == RunState::Playing {
            let period = (1.0 / self.tick_hz).clamp(1.0 / 240.0, 1.0);
            ctx.request_repaint_after(Duration::from_secs_f32(period));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ticks_to_run_state_machine() {
        let mut acc = 0.0;
        assert_eq!(ticks_to_run(RunState::Stopped, 10.0, 1.0, &mut acc), 0);
        assert_eq!(ticks_to_run(RunState::Stepping, 10.0, 1.0, &mut acc), 1);

        acc = 0.0;
        // 10 Hz, 0.25 s elapsed → 2 ticks, 0.05 s carried.
        assert_eq!(ticks_to_run(RunState::Playing, 10.0, 0.25, &mut acc), 2);
        assert!((acc - 0.05).abs() < 1e-6);
        // tiny dts accumulate rather than dropping ticks
        acc = 0.0;
        assert_eq!(ticks_to_run(RunState::Playing, 10.0, 0.04, &mut acc), 0);
        assert_eq!(ticks_to_run(RunState::Playing, 10.0, 0.07, &mut acc), 1);
    }

    #[test]
    fn log_ring_caps_and_drops_oldest() {
        let mut r = LogRing::new(3);
        for tick in 0..5u64 {
            r.extend([Diagnostic {
                level: octans_core::LogLevel::Info,
                source: "t".into(),
                message: "m".into(),
                tick,
            }]);
        }
        assert_eq!(r.len(), 3);
        assert_eq!(r.iter().next().unwrap().tick, 2); // 0 and 1 dropped
        r.clear();
        assert!(r.is_empty());
    }
}
