//! Octans GUI — a read-only "watch it run" visualizer for an Octans graph.
//!
//! All logic lives in the library (and sibling modules) so it compiles and unit-tests headlessly.
//! The only window-opening call (`eframe::run_native`) lives in `src/main.rs`, which `cargo
//! test`/`clippy` compile but never execute — so CI never needs a display.

pub mod canvas;
pub mod layout;
pub mod log_panel;
pub mod model;
pub mod profiler;
pub mod scene;

use eframe::egui;
use octans_core::{Diagnostic, Fault, Graph, Mira, NodeId, Tick};
use scene::SceneKind;
use std::collections::{HashSet, VecDeque};
use std::time::Duration;

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
    pub(crate) engine: Mira,
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
    pub(crate) profiler_sort: ProfileSort,
}

impl OctansApp {
    pub fn new(_cc: &eframe::CreationContext<'_>, kind: SceneKind) -> Self {
        let (graph, engine) = kind.build();
        let view = model::ViewGraph::from_graph(&graph);
        let layout = layout::layout(&view);
        Self {
            graph,
            engine,
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
            profiler_sort: ProfileSort::default(),
        }
    }

    /// Rebuild everything for a fresh scene.
    pub fn set_scene(&mut self, kind: SceneKind) {
        let (graph, engine) = kind.build();
        self.view = model::ViewGraph::from_graph(&graph);
        self.layout = layout::layout(&self.view);
        self.graph = graph;
        self.engine = engine;
        self.scene_kind = kind;
        self.run = RunState::Stopped;
        self.accumulator = 0.0;
        self.tick_count = 0;
        self.last_tick = None;
        self.log.clear();
        self.camera = Camera::default();
    }

    /// Fold one tick's results into the panels' state.
    pub fn ingest_tick(&mut self, tick: Tick) {
        self.log.extend(tick.diagnostics.iter().cloned());
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
            }
            ui.separator();

            let mut kind = self.scene_kind;
            egui::ComboBox::from_label("scene")
                .selected_text(kind.label())
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut kind, SceneKind::Tracker, "tracker");
                    ui.selectable_value(&mut kind, SceneKind::Diagnostics, "diagnostics");
                });
            if kind != self.scene_kind {
                self.set_scene(kind);
            }

            ui.separator();
            ui.checkbox(&mut self.show_latency_overlay, "latency overlay");
        });
    }
}

impl eframe::App for OctansApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Advance the engine.
        let dt = ctx.input(|i| i.stable_dt);
        let n = ticks_to_run(self.run, self.tick_hz, dt, &mut self.accumulator);
        for _ in 0..n {
            let tick = self.engine.run_tick(&self.graph);
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
        egui::CentralPanel::default().show(ctx, |ui| self.canvas_ui(ui));

        // Keep animating while playing (egui is reactive and would otherwise idle).
        if self.run == RunState::Playing {
            ctx.request_repaint();
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
