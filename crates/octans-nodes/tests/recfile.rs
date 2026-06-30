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

/// The detect pipeline as a reusable group: a frame -> threshold -> centroid (normalized obs).
fn detect_group(w: usize, h: usize, f: f64) -> octans_core::GroupTemplate {
    group("detect", move |gg| {
        let t = gg.add(Threshold);
        let c = gg.add(Centroid { w, h, f });
        gg.connect(t, "mask", c, "mask");
        gg.input("frame", t, "image");
        gg.output("px", c, "px");
    })
}

#[test]
fn record_then_replay_the_tracker_recovers_identical_points() {
    let (w, h, f) = (64usize, 64usize, 100.0f64);
    let centers = [[-1.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, -1.0, 0.0]];
    let cam_chan = ["cam0", "cam1", "cam2"];
    let path = record_path("octans_tracker.rec");
    let _ = std::fs::remove_file(&path);

    // --- phase 1: run the LIVE tracker, recording each camera's frame + the recovered point ---
    let mut live_points: Vec<[f64; 3]> = Vec::new();
    {
        let mut reg = Registry::new();
        register_primitives(&mut reg);
        register_node_types(&mut reg);
        register_tracking_types(&mut reg);
        let mut g = Graph::new(reg);

        let pt = g.add(MovingPoint {
            start: [0.0, 0.0, 5.0],
            vel: [0.05, 0.0, 0.0],
        });
        let detect = detect_group(w, h, f);
        let mut rec = Recorder::new(&path).channel::<Pt3>("point");
        for c in &cam_chan {
            rec = rec.channel::<Image>(c);
        }
        let recorder = g.add(rec);

        let mut px_outs = Vec::new();
        for (i, &center) in centers.iter().enumerate() {
            let sim = g.add(CameraSim { center, w, h, f });
            g.connect(pt, "point", sim, "point").unwrap();
            // record this camera's raw frame
            g.connect(sim, "frame", recorder, cam_chan[i]).unwrap();
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
        g.connect(tri, "point", recorder, "point").unwrap();

        let mut engine = Mira::compile(&g).unwrap();
        for _ in 0..5 {
            let t = engine.run_tick(&g);
            let p = t
                .output(tri, "point")
                .unwrap()
                .downcast_ref::<Pt3>()
                .unwrap()
                .0;
            live_points.push(p);
        }
    }

    // --- phase 2: REPLAY the recorded frames through detect+triangulate (no live cameras) ---
    let replayer = Replayer::open(&path).expect("open tracker recording");

    let mut reg = Registry::new();
    register_primitives(&mut reg);
    register_node_types(&mut reg);
    register_tracking_types(&mut reg);
    let mut g = Graph::new(reg);
    let rep = g.add(replayer);
    let detect = detect_group(w, h, f);

    let mut px_outs = Vec::new();
    for chan in &cam_chan {
        let d = g.add_group(&detect).unwrap();
        let (fnode, fport) = d.input("frame");
        g.connect(rep, chan, fnode, fport).unwrap(); // replayed frame -> detect
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

    let mut engine = Mira::compile(&g).unwrap();
    for (i, live) in live_points.iter().enumerate() {
        let t = engine.run_tick(&g);
        let replayed = t
            .output(tri, "point")
            .unwrap()
            .downcast_ref::<Pt3>()
            .unwrap()
            .0;
        let recorded = t
            .output(rep, "point")
            .unwrap()
            .downcast_ref::<Pt3>()
            .unwrap()
            .0;
        // Replaying the recorded u8 frames (which round-trip exactly) through the same pipeline
        // reproduces the recovered point, and it matches the point recorded alongside them. The
        // JSON text format preserves f64 to full precision but not always bit-identically, so we
        // compare within tolerance (the observed wobble is ~1e-31 in the ~0 axis).
        for k in 0..3 {
            assert!(
                (replayed[k] - live[k]).abs() < 1e-9 && (recorded[k] - live[k]).abs() < 1e-9,
                "tick {i}: replayed {replayed:?} / recorded {recorded:?} vs live {live:?}"
            );
        }
        // Sanity: it actually tracks the moving ground truth (x = 0.05·tick, y≈0, z=5).
        let expected = [0.05 * (i as f64 + 1.0), 0.0, 5.0];
        for k in 0..3 {
            assert!((replayed[k] - expected[k]).abs() < 0.02);
        }
    }

    let _ = std::fs::remove_file(&path);
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

#[test]
fn replayer_with_explicit_schema_reads_a_headered_file() {
    let path = record_path("octans_with_schema.rec");
    let _ = std::fs::remove_file(&path);

    // Record a single u32 channel.
    {
        let mut reg = Registry::new();
        register_primitives(&mut reg);
        let mut g = Graph::new(reg);
        let scene = g.add(Scene);
        let rec = g.add(Recorder::new(&path).channel::<u32>("count"));
        g.connect(scene, "count", rec, "count").unwrap();
        let mut engine = Mira::compile(&g).unwrap();
        for _ in 0..3 {
            engine.run_tick(&g);
        }
    }

    // Replay with an author-declared schema (path "a") rather than reading the header.
    let mut reg = Registry::new();
    register_primitives(&mut reg);
    let mut g = Graph::new(reg);
    let rep = g.add(Replayer::with_schema(&path, &[("count", "octans.u32")]));
    let mut engine = Mira::compile(&g).unwrap();

    for expect in 1u32..=3 {
        let t = engine.run_tick(&g);
        assert_eq!(
            t.output(rep, "count").unwrap().downcast_ref::<u32>(),
            Some(&expect)
        );
    }

    let _ = std::fs::remove_file(&path);
}
