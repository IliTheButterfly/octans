//! End-to-end: a moving 3D point is observed by 3 synthetic cameras, each frame runs a detect
//! pipeline (threshold -> centroid), the centroids are gathered and triangulated, and the
//! recovered point must track the ground truth — every tick, through the whole engine.

use octans_core::*;
use octans_nodes::*;

#[test]
fn end_to_end_tracker_recovers_the_moving_point() {
    let (w, h, f) = (256usize, 256usize, 400.0f64);
    let centers = [[-1.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, -1.0, 0.0]];

    let mut reg = Registry::new();
    register_primitives(&mut reg);
    register_node_types(&mut reg); // Image
    register_tracking_types(&mut reg); // Proj / Px / Pt3
    let mut g = Graph::new(reg);

    // Ground-truth point, drifting along +x.
    let pt = g.add(MovingPoint {
        start: [0.0, 0.0, 5.0],
        vel: [0.05, 0.0, 0.0],
    });

    // Per-camera detection pipeline: threshold -> centroid (normalized observation).
    let detect = group("detect", move |gg| {
        let t = gg.add(Threshold);
        let c = gg.add(Centroid { w, h, f });
        gg.connect(t, "mask", c, "mask");
        gg.input("frame", t, "image");
        gg.output("px", c, "px");
    });

    // One CameraSim + detect per camera; all observe the same moving point.
    let mut px_outs = Vec::new();
    for &center in &centers {
        let sim = g.add(CameraSim { center, w, h, f });
        g.connect(pt, "point", sim, "point").unwrap();
        let d = g.add_group(&detect).unwrap();
        let (fnode, fport) = d.input("frame");
        g.connect(sim, "frame", fnode, fport).unwrap();
        px_outs.push(d.output("px"));
    }

    // Gather the 3 observations and triangulate against the cameras' projection matrices.
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
    for tick in 1..=5u64 {
        let res = engine.run_tick(&g);
        let p = res
            .output(tri, "point")
            .unwrap()
            .downcast_ref::<Pt3>()
            .unwrap()
            .0;
        let expected = [0.05 * tick as f64, 0.0, 5.0];
        for k in 0..3 {
            assert!(
                (p[k] - expected[k]).abs() < 0.05,
                "tick {tick}: recovered {p:?} vs expected {expected:?}"
            );
        }
    }
}

#[test]
fn tracker_degrades_gracefully_when_the_target_is_not_visible() {
    let (w, h, f) = (256usize, 256usize, 400.0f64);
    let centers = [[-1.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, -1.0, 0.0]];

    let mut reg = Registry::new();
    register_primitives(&mut reg);
    register_node_types(&mut reg);
    register_tracking_types(&mut reg);
    let mut g = Graph::new(reg);

    // Point sits *behind* the cameras (negative z): no camera renders it.
    let pt = g.add(MovingPoint {
        start: [0.0, 0.0, -5.0],
        vel: [0.0, 0.0, 0.0],
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

    let mut engine = Mira::compile(&g).unwrap();
    let res = engine.run_tick(&g);

    // No detection anywhere -> centroids return None -> Gather and Triangulate skip. The tick
    // completes cleanly: no panic, no fault, and crucially no bogus recovered point.
    assert!(res.ok(), "no node should fault: {:?}", res.faults);
    assert!(res.output(tri, "point").is_none());
    assert!(res.skipped.contains(&gather));
    assert!(res.skipped.contains(&tri));
}

#[test]
fn triangulate_with_a_single_view_produces_no_point() {
    let mut reg = Registry::new();
    register_primitives(&mut reg);
    register_tracking_types(&mut reg);
    let mut g = Graph::new(reg);

    // Only one camera/observation: a 3D point is under-determined.
    let projs = g.add(VecConst {
        items: vec![Proj::camera([0.0, 0.0, 0.0])],
    });
    let pxs = g.add(VecConst {
        items: vec![Px([0.1, 0.1])],
    });
    let tri = g.add(Triangulate);
    g.connect(projs, "items", tri, "proj").unwrap();
    g.connect(pxs, "items", tri, "px").unwrap();

    let mut engine = Mira::compile(&g).unwrap();
    let res = engine.run_tick(&g);
    assert!(res.ok());
    // Under-determined: Triangulate runs but declines to emit a point rather than fabricate one.
    assert!(res.output(tri, "point").is_none());
}
