//! Octans GUI — a read-only "watch it run" visualizer for an Octans graph.
//!
//! All logic lives in the library (and sibling modules) so it compiles and unit-tests headlessly.
//! The only window-opening call (`eframe::run_native`) lives in `src/main.rs`, which `cargo
//! test`/`clippy` compile but never execute — so CI never needs a display.

pub mod canvas;
pub mod inspector;
pub mod layout;
pub mod log_panel;
pub mod model;
pub mod palette;
pub mod profiler;
pub mod pyxis;
pub mod scene;
pub mod schedule;

use eframe::egui;
use octans_core::{
    Catalog, Diagnostic, Fault, Graph, Mira, NodeId, StrategyHandle, Tick, TuneResult, Value,
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
}

impl OctansApp {
    pub fn new(_cc: &eframe::CreationContext<'_>, kind: SceneKind) -> Self {
        let scene = kind.build();
        let view = model::ViewGraph::from_graph(&scene.graph);
        let layout = layout::layout(&view);
        let mut catalog = Catalog::new();
        octans_nodes::register_std_catalog(&mut catalog);
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
        }
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
                self.graph.disconnect_input(to, to_port);
                let _ = self.graph.connect(from, from_port, to, to_port);
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
        self.layout = layout::layout(&self.view);
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
        self.layout = layout::layout(&self.view);
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
        ui.horizontal(|ui| {
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
        });
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
