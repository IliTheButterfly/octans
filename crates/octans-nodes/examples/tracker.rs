//! End-to-end tracker demo: a moving 3D point observed by 3 cameras, detected, and triangulated
//! back — through the whole engine. Prints the recovered point per tick and the graph as DOT.
//! Run: `cargo run -p octans-nodes --example tracker`

use octans_core::*;
use octans_nodes::*;

fn main() {
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

    let mut engine = Mira::compile(&g).unwrap();
    println!("tracking a moving point through 3 cameras → detect → triangulate:\n");
    for tick in 1..=5u64 {
        let res = engine.run_tick(&g);
        let p = res
            .output(tri, "point")
            .unwrap()
            .downcast_ref::<Pt3>()
            .unwrap()
            .0;
        println!(
            "  tick {tick}: recovered [{:+.3}, {:+.3}, {:+.3}]   truth [{:+.3}, +0.000, +5.000]   ({:?})",
            p[0],
            p[1],
            p[2],
            0.05 * tick as f64,
            res.latency,
        );
    }
    println!("\nnode count: {}", g.to_spec().nodes.len());
}
