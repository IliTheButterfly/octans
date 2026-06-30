//! Record/replay: a multi-type stream written by Recorder must replay identically through a
//! Replayer whose ports are populated from the file's own header.

use octans_core::*;
use octans_nodes::*;
use std::any::Any;

/// Emits a u32 count and a Pt3 that both depend on the tick, so each frame is distinct.
struct Scene;
impl Node for Scene {
    fn node_type(&self) -> &'static str {
        "test.scene"
    }
    fn inputs(&self) -> Vec<PortSpec> {
        vec![]
    }
    fn outputs(&self) -> Vec<PortSpec> {
        vec![
            PortSpec::new("count", u32::type_spec()),
            PortSpec::new("point", Pt3::type_spec()),
        ]
    }
    fn process(&self, c: &Context, _l: &mut dyn Any, _i: &Inputs, o: &mut Outputs) {
        let t = c.tick();
        o.set("count", t as u32);
        o.set("point", Pt3([t as f64 * 0.5, 1.0, -2.0]));
    }
}

fn record_path(name: &str) -> std::path::PathBuf {
    let dir = std::env::var("CARGO_TARGET_TMPDIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir());
    dir.join(name)
}

#[test]
fn record_then_replay_reproduces_a_multi_type_stream() {
    let path = record_path("octans_roundtrip.rec");
    let _ = std::fs::remove_file(&path);

    // --- record 4 ticks ---
    let mut expected: Vec<(u32, [f64; 3])> = Vec::new();
    {
        let mut reg = Registry::new();
        register_primitives(&mut reg);
        register_tracking_types(&mut reg);
        let mut g = Graph::new(reg);
        let scene = g.add(Scene);
        let rec = g.add(
            Recorder::new(&path)
                .channel::<u32>("count")
                .channel::<Pt3>("point"),
        );
        g.connect(scene, "count", rec, "count").unwrap();
        g.connect(scene, "point", rec, "point").unwrap();

        let mut engine = Mira::compile(&g).unwrap();
        for _ in 0..4 {
            let t = engine.run_tick(&g);
            let c = *t
                .output(scene, "count")
                .unwrap()
                .downcast_ref::<u32>()
                .unwrap();
            let p = t
                .output(scene, "point")
                .unwrap()
                .downcast_ref::<Pt3>()
                .unwrap()
                .0;
            expected.push((c, p));
        }
    } // graph (and recorder file handle) dropped here

    // --- replay: ports come from the file header, not restated by us ---
    let replayer = Replayer::open(&path).expect("open record file");
    assert_eq!(
        replayer.outputs().len(),
        2,
        "Replayer should expose both channels from the file schema"
    );

    let mut reg = Registry::new();
    register_primitives(&mut reg);
    register_tracking_types(&mut reg);
    let mut g = Graph::new(reg);
    let rep = g.add(replayer);
    let mut engine = Mira::compile(&g).unwrap();

    for (c, p) in &expected {
        let t = engine.run_tick(&g);
        let rc = *t
            .output(rep, "count")
            .unwrap()
            .downcast_ref::<u32>()
            .unwrap();
        let rp = t
            .output(rep, "point")
            .unwrap()
            .downcast_ref::<Pt3>()
            .unwrap()
            .0;
        assert_eq!(rc, *c);
        assert_eq!(rp, *p);
    }

    // Past the end of the recording: the replayer emits nothing.
    let t = engine.run_tick(&g);
    assert!(t.output(rep, "count").is_none());

    let _ = std::fs::remove_file(&path);
}
