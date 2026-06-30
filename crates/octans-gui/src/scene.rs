//! Example scenes, built in code (there's no editor yet). Each returns a compiled `(Graph, Mira)`
//! ready to tick — mirroring `octans-nodes/examples/{tracker,diagnostics}.rs`.

use octans_core::{
    group, register_primitives, Context, Gather, Graph, Inputs, Mira, Node, NodeId, Outputs,
    PortSpec, RegisteredType, Registry, Strategy, StrategyHandle,
};
use octans_nodes::*;
use std::any::Any;

/// A built scene: the graph + compiled engine, plus any `Strategy` nodes' handles (for the
/// autotuner / live A/B).
pub struct Scene {
    pub graph: Graph,
    pub engine: Mira,
    pub strategies: Vec<(NodeId, StrategyHandle)>,
}

/// Which built-in scene to show.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SceneKind {
    Tracker,
    Diagnostics,
    Robustness,
    Strategy,
}

impl SceneKind {
    pub const ALL: [SceneKind; 4] = [
        SceneKind::Tracker,
        SceneKind::Diagnostics,
        SceneKind::Robustness,
        SceneKind::Strategy,
    ];

    pub fn label(self) -> &'static str {
        match self {
            SceneKind::Tracker => "tracker",
            SceneKind::Diagnostics => "diagnostics",
            SceneKind::Robustness => "robustness",
            SceneKind::Strategy => "strategy",
        }
    }
    pub fn build(self) -> Scene {
        let (graph, engine, strategies) = match self {
            SceneKind::Tracker => {
                let (g, e) = tracker();
                (g, e, vec![])
            }
            SceneKind::Diagnostics => {
                let (g, e) = diagnostics();
                (g, e, vec![])
            }
            SceneKind::Robustness => {
                let (g, e) = robustness();
                (g, e, vec![])
            }
            SceneKind::Strategy => strategy_scene(),
        };
        Scene {
            graph,
            engine,
            strategies,
        }
    }
}

/// Multi-camera tracker: MovingPoint → 3×(CameraSim → detect group) → Gather → Triangulate.
pub fn tracker() -> (Graph, Mira) {
    let (w, h, f) = (256usize, 256usize, 400.0f64);
    let centers = [[-1.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, -1.0, 0.0]];

    let mut reg = Registry::new();
    register_primitives(&mut reg);
    register_node_types(&mut reg);
    register_tracking_types(&mut reg);
    let mut g = Graph::new(reg);

    let pt = g.add(MovingPoint {
        start: [0.0, 0.0, 5.0],
        vel: [0.05, 0.0, 0.0],
    });
    let detect = group("detect", move |gg| {
        let t = gg.add(Threshold);
        let c = gg.add(Centroid { w, h, f });
        gg.connect(t, "mask", c, "mask");
        gg.input("frame", t, "image");
        gg.output("px", c, "px");
    });

    let mut px_outs = Vec::new();
    for &center in &centers {
        let sim = g.add(CameraSim { center, w, h, f });
        g.connect(pt, "point", sim, "point").unwrap();
        let d = g.add_group(&detect).unwrap();
        let (fnode, fport) = d.input("frame");
        g.connect(sim, "frame", fnode, fport).unwrap();
        px_outs.push(d.output("px"));
    }
    let gather = g.add(Gather::new::<Px>(3));
    for (i, (onode, oport)) in px_outs.iter().enumerate() {
        g.connect(*onode, oport, gather, &format!("in{i}")).unwrap();
    }
    let projs = g.add(VecConst {
        items: centers.iter().map(|&c| Proj::camera(c)).collect::<Vec<_>>(),
    });
    let tri = g.add(Triangulate);
    g.connect(projs, "items", tri, "proj").unwrap();
    g.connect(gather, "items", tri, "px").unwrap();

    let engine = Mira::compile(&g).expect("tracker scene compiles");
    (g, engine)
}

/// Diagnostics demo: camera → threshold → blob-count → Probe → Log + LogFmt (exercises the log).
pub fn diagnostics() -> (Graph, Mira) {
    let mut reg = Registry::new();
    register_primitives(&mut reg);
    register_node_types(&mut reg);
    let mut g = Graph::new(reg);

    let cam = g.add(SyntheticCamera {
        w: 64,
        h: 64,
        blobs: vec![(16, 16, 5), (40, 40, 7)],
    });
    let thr = g.add(Threshold);
    let blobs = g.add(BlobCount);
    let probe = g.add(Probe::<u32>::new("blob-count"));
    let logger = g.add(Log::<u32>::warning("vision"));
    let report = g.add(LogFmt::info("vision", "frame had {{count}} blobs").arg::<u32>("count"));

    g.connect(cam, "frame", thr, "image").unwrap();
    g.connect(thr, "mask", blobs, "mask").unwrap();
    g.connect(blobs, "count", probe, "in").unwrap();
    g.connect(probe, "out", logger, "value").unwrap();
    g.connect(probe, "out", report, "count").unwrap();

    let engine = Mira::compile(&g).expect("diagnostics scene compiles");
    (g, engine)
}

/// A robustness showcase: a node that always panics (→ fault, isolated), nodes skipped by the
/// resulting missing input (fault cascade), an intermittent source whose consumers skip on even
/// ticks (skip cascade), and a healthy chain for contrast. Drives the fault/skip visualization.
pub fn robustness() -> (Graph, Mira) {
    let mut reg = Registry::new();
    register_primitives(&mut reg);
    let mut g = Graph::new(reg);

    // healthy chain (stays green)
    let c = g.add(Const42);
    let e1 = g.add(EchoI32);
    g.connect(c, "out", e1, "in").unwrap();

    // a faulting node and its downstream (skipped because the bomb produced nothing)
    let bomb = g.add(Bomb);
    let e2 = g.add(EchoI32);
    g.connect(bomb, "out", e2, "in").unwrap();

    // intermittent source → two-stage skip cascade on even ticks
    let odd = g.add(OddOnly);
    let e3 = g.add(EchoI32);
    let e4 = g.add(EchoI32);
    g.connect(odd, "out", e3, "in").unwrap();
    g.connect(e3, "out", e4, "in").unwrap();

    let engine = Mira::compile(&g).expect("robustness scene compiles");
    (g, engine)
}

/// An autotuner showcase: a camera feeds a `Strategy` with two interchangeable, bit-identical
/// detect implementations — a two-pass `Threshold→Centroid` group and a fused `ThresholdCentroid`
/// node. Pyxis benchmarks them on the live hardware and picks the faster verified one.
pub fn strategy_scene() -> (Graph, Mira, Vec<(NodeId, StrategyHandle)>) {
    let (w, h, f) = (64usize, 64usize, 100.0f64);
    let mut reg = Registry::new();
    register_primitives(&mut reg);
    register_node_types(&mut reg);
    register_tracking_types(&mut reg);
    let mut g = Graph::new(reg);

    let cam = g.add(SyntheticCamera {
        w,
        h,
        blobs: vec![(20, 20, 6), (44, 40, 5)],
    });
    let two_pass = group("two_pass", move |gg| {
        let t = gg.add(Threshold);
        let c = gg.add(Centroid { w, h, f });
        gg.connect(t, "mask", c, "mask");
        gg.input("frame", t, "image");
        gg.output("px", c, "px");
    });
    let strat = Strategy::builder()
        .group("two_pass", &two_pass)
        .node("fused", ThresholdCentroid { w, h, f, thr: 128 })
        .build();
    let handle = strat.handle();
    let s = g.add(strat);
    g.connect(cam, "frame", s, "frame").unwrap();
    let log = g.add(Log::<Px>::info("detect"));
    g.connect(s, "px", log, "value").unwrap();

    let engine = Mira::compile(&g).expect("strategy scene compiles");
    (g, engine, vec![(s, handle)])
}

// --- tiny demo nodes for the robustness scene ---

struct Const42;
impl Node for Const42 {
    fn node_type(&self) -> &'static str {
        "demo.const42"
    }
    fn inputs(&self) -> Vec<PortSpec> {
        vec![]
    }
    fn outputs(&self) -> Vec<PortSpec> {
        vec![PortSpec::new("out", i32::type_spec())]
    }
    fn process(&self, _c: &Context, _l: &mut dyn Any, _i: &Inputs, o: &mut Outputs) {
        o.set("out", 42i32);
    }
}

struct Bomb;
impl Node for Bomb {
    fn node_type(&self) -> &'static str {
        "demo.bomb"
    }
    fn inputs(&self) -> Vec<PortSpec> {
        vec![]
    }
    fn outputs(&self) -> Vec<PortSpec> {
        vec![PortSpec::new("out", i32::type_spec())]
    }
    fn process(&self, _c: &Context, _l: &mut dyn Any, _i: &Inputs, _o: &mut Outputs) {
        panic!("boom (intentional: demonstrates fault isolation)");
    }
}

struct OddOnly;
impl Node for OddOnly {
    fn node_type(&self) -> &'static str {
        "demo.odd_only"
    }
    fn inputs(&self) -> Vec<PortSpec> {
        vec![]
    }
    fn outputs(&self) -> Vec<PortSpec> {
        vec![PortSpec::new("out", i32::type_spec())]
    }
    fn process(&self, c: &Context, _l: &mut dyn Any, _i: &Inputs, o: &mut Outputs) {
        if c.tick() % 2 == 1 {
            o.set("out", 1i32);
        }
    }
}

struct EchoI32;
impl Node for EchoI32 {
    fn node_type(&self) -> &'static str {
        "demo.echo"
    }
    fn inputs(&self) -> Vec<PortSpec> {
        vec![PortSpec::new("in", i32::type_spec())]
    }
    fn outputs(&self) -> Vec<PortSpec> {
        vec![PortSpec::new("out", i32::type_spec())]
    }
    fn process(&self, _c: &Context, _l: &mut dyn Any, i: &Inputs, o: &mut Outputs) {
        o.set_value("out", i.value("in").clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graph_scenes_compile_and_tick_headlessly() {
        for kind in [
            SceneKind::Tracker,
            SceneKind::Diagnostics,
            SceneKind::Strategy,
        ] {
            let mut s = kind.build();
            assert!(s.graph.node_count() > 0);
            for _ in 0..3 {
                let tick = s.engine.run_tick(&s.graph);
                assert!(tick.ok(), "{} faulted: {:?}", kind.label(), tick.faults);
            }
        }
    }

    #[test]
    fn strategy_scene_exposes_a_tunable_handle() {
        let mut s = SceneKind::Strategy.build();
        assert_eq!(s.strategies.len(), 1);
        let (node, handle) = s.strategies[0].clone();
        assert_eq!(handle.variant_count(), 2);
        let res = s.engine.tune(
            &s.graph,
            &[(node, handle)],
            octans_core::TuneConfig {
                warmup: 1,
                trials: 2,
            },
        );
        assert_eq!(res.len(), 1);
        assert!(
            res[0].rejected.is_empty(),
            "the two variants are equivalent"
        );
    }

    #[test]
    fn robustness_scene_produces_faults_and_skips() {
        // Silence the intentional Bomb panic during the test.
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let mut s = SceneKind::Robustness.build();
        let _ = s.engine.run_tick(&s.graph); // tick 1 is odd; run one to land on tick 2 (even)
        let t2 = s.engine.run_tick(&s.graph);
        std::panic::set_hook(prev);

        assert_eq!(t2.faults.len(), 1, "the bomb faults");
        // bomb's consumer + the intermittent cascade skip on the even tick
        assert!(!t2.skipped.is_empty(), "skips cascade");
    }

    #[test]
    fn diagnostics_emits_log_lines() {
        let mut s = SceneKind::Diagnostics.build();
        let tick = s.engine.run_tick(&s.graph);
        assert!(
            !tick.diagnostics.is_empty(),
            "diagnostics scene should log each tick"
        );
    }
}
