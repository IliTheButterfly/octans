//! Octans doing its actual job: recover a 3D point from several cameras' 2D observations.

use octans_core::*;
use octans_nodes::*;

/// Project a world point into camera at position `c` (pinhole, P = [I | -c]).
fn project(c: [f64; 3], x: [f64; 3]) -> Px {
    let d = [x[0] - c[0], x[1] - c[1], x[2] - c[2]];
    Px([d[0] / d[2], d[1] / d[2]])
}

#[test]
fn triangulate_recovers_a_3d_point() {
    let x0 = [1.0, 2.0, 3.0];
    let cams = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];

    let projs: Vec<Proj> = cams.iter().map(|&c| Proj::camera(c)).collect();
    let pxs: Vec<Px> = cams.iter().map(|&c| project(c, x0)).collect();

    let mut reg = Registry::new();
    register_primitives(&mut reg);
    register_tracking_types(&mut reg);
    let mut g = Graph::new(reg);

    let psrc = g.add(VecConst { items: projs });
    let xsrc = g.add(VecConst { items: pxs });
    let tri = g.add(Triangulate);
    g.connect(psrc, "items", tri, "proj").unwrap();
    g.connect(xsrc, "items", tri, "px").unwrap();

    let mut engine = Mira::compile(&g).unwrap();
    let pt = engine
        .run_tick(&g)
        .output(tri, "point")
        .unwrap()
        .downcast_ref::<Pt3>()
        .unwrap()
        .0;

    for k in 0..3 {
        assert!(
            (pt[k] - x0[k]).abs() < 1e-6,
            "recovered {pt:?} should equal {x0:?}"
        );
    }
}
