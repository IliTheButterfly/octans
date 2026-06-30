//! Record/replay the tracker: run the live multi-camera tracker while recording each camera's
//! raw frames to a file, then replay those frames through the detect+triangulate pipeline with no
//! live cameras at all — recovering the same trajectory deterministically.
//!
//! Run: `cargo run -p octans-nodes --example tracker_replay`

use octans_core::*;
use octans_nodes::*;

fn detect_group(w: usize, h: usize, f: f64) -> GroupTemplate {
    group("detect", move |gg| {
        let t = gg.add(Threshold);
        let c = gg.add(Centroid { w, h, f });
        gg.connect(t, "mask", c, "mask");
        gg.input("frame", t, "image");
        gg.output("px", c, "px");
    })
}

fn main() {
    let (w, h, f) = (64usize, 64usize, 100.0f64);
    let centers = [[-1.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, -1.0, 0.0]];
    let cam_chan = ["cam0", "cam1", "cam2"];
    let path = std::env::temp_dir().join("octans_tracker_demo.rec");

    // --- phase 1: live tracker, recording raw camera frames ---
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
        let mut rec = Recorder::new(&path);
        for c in &cam_chan {
            rec = rec.channel::<Image>(c);
        }
        let recorder = g.add(rec);
        let mut px_outs = Vec::new();
        for (i, &center) in centers.iter().enumerate() {
            let sim = g.add(CameraSim { center, w, h, f });
            g.connect(pt, "point", sim, "point").unwrap();
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

        let mut engine = Mira::compile(&g).unwrap();
        for _ in 0..5 {
            engine.run_tick(&g);
        }
        println!(
            "recorded 5 ticks of 3 camera frames to {}\n",
            path.display()
        );
    }

    // --- phase 2: replay the frames through detect+triangulate (no cameras) ---
    let replayer = Replayer::open(&path).expect("open recording");
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
        g.connect(rep, chan, fnode, fport).unwrap();
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
    println!("replaying frames → detect → triangulate (no live cameras):");
    for _ in 0..5 {
        let t = engine.run_tick(&g);
        match t.output(tri, "point") {
            Some(v) => {
                let p = v.downcast_ref::<Pt3>().unwrap().0;
                println!("  recovered [{:+.3}, {:+.3}, {:+.3}]", p[0], p[1], p[2]);
            }
            None => println!("  (no point this frame)"),
        }
    }

    let _ = std::fs::remove_file(&path);
}
