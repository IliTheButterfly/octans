//! Example scenes, built in code (there's no editor yet). Each returns a compiled `(Graph, Mira)`
//! ready to tick — mirroring `octans-nodes/examples/{tracker,diagnostics}.rs`.

use octans_core::{group, register_primitives, Gather, Graph, Mira, Registry};
use octans_nodes::*;

/// Which built-in scene to show.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SceneKind {
    Tracker,
    Diagnostics,
}

impl SceneKind {
    pub fn label(self) -> &'static str {
        match self {
            SceneKind::Tracker => "tracker",
            SceneKind::Diagnostics => "diagnostics",
        }
    }
    pub fn build(self) -> (Graph, Mira) {
        match self {
            SceneKind::Tracker => tracker(),
            SceneKind::Diagnostics => diagnostics(),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_scenes_compile_and_tick_headlessly() {
        for kind in [SceneKind::Tracker, SceneKind::Diagnostics] {
            let (graph, mut engine) = kind.build();
            assert!(graph.node_count() > 0);
            for _ in 0..3 {
                let tick = engine.run_tick(&graph);
                assert!(tick.ok(), "{} faulted: {:?}", kind.label(), tick.faults);
            }
        }
    }

    #[test]
    fn diagnostics_emits_log_lines() {
        let (graph, mut engine) = SceneKind::Diagnostics.build();
        let tick = engine.run_tick(&graph);
        assert!(
            !tick.diagnostics.is_empty(),
            "diagnostics scene should log each tick"
        );
    }
}
