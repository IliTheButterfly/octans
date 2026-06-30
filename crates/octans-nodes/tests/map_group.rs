//! `Map` over a *group*: fan a whole pipeline (Threshold -> BlobCount) over N camera frames,
//! each lane running the pipeline independently. This is the core of the multi-camera shape.

use octans_core::*;
use octans_nodes::*;
use std::any::Any;

fn frame_with_blobs(w: usize, h: usize, blobs: &[(i32, i32, i32)]) -> Image {
    let mut px = vec![30u8; w * h];
    for &(cx, cy, r) in blobs {
        let r2 = r * r;
        for y in (cy - r).max(0)..(cy + r + 1).min(h as i32) {
            for x in (cx - r).max(0)..(cx + r + 1).min(w as i32) {
                let (dx, dy) = (x - cx, y - cy);
                if dx * dx + dy * dy <= r2 {
                    px[y as usize * w + x as usize] = 220;
                }
            }
        }
    }
    Image { w, h, px }
}

/// A source emitting a fixed `Vector<Image>` (hand-written: vector outputs aren't derived by
/// `#[node]` yet).
struct VecCamera {
    frames: Vec<Image>,
}
impl Node for VecCamera {
    fn node_type(&self) -> &'static str {
        "test.vec_camera"
    }
    fn inputs(&self) -> Vec<PortSpec> {
        Vec::new()
    }
    fn outputs(&self) -> Vec<PortSpec> {
        vec![PortSpec::new(
            "frames",
            TypeSpec {
                id: T_IMAGE,
                shape: Shape::Vector(None),
            },
        )]
    }
    fn process(&self, _c: &Context, _l: &mut dyn Any, _i: &Inputs, out: &mut Outputs) {
        out.set_value(
            "frames",
            Value::vector(self.frames.iter().cloned().map(Value::new).collect()),
        );
    }
}

#[test]
fn map_over_group_fans_a_pipeline_per_frame() {
    // A reusable detection pipeline as a group.
    let detect = group("detect", |g| {
        let t = g.add(Threshold);
        let b = g.add(BlobCount);
        g.connect(t, "mask", b, "mask");
        g.input("image", t, "image");
        g.output("count", b, "count");
    });

    let mut reg = Registry::new();
    register_primitives(&mut reg);
    register_node_types(&mut reg);
    let mut graph = Graph::new(reg);

    let cam = graph.add(VecCamera {
        frames: vec![
            frame_with_blobs(96, 96, &[(20, 20, 6), (70, 70, 7)]), // 2 blobs
            frame_with_blobs(96, 96, &[(20, 20, 6), (70, 20, 6), (45, 70, 6)]), // 3 blobs
        ],
    });
    let m = graph.add(Map::group(&detect)); // fan the whole pipeline per frame
    graph.connect(cam, "frames", m, "items").unwrap();

    let mut engine = Mira::compile(&graph).unwrap();
    let tick = engine.run_tick(&graph);

    let counts: Vec<u32> = tick
        .output(m, "items")
        .unwrap()
        .as_vector()
        .unwrap()
        .iter()
        .map(|v| *v.downcast_ref::<u32>().unwrap())
        .collect();

    assert_eq!(
        counts,
        vec![2, 3],
        "each frame ran the full detect pipeline independently"
    );
}
