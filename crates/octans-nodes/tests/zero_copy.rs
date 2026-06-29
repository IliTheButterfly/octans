//! Fan-out is zero-copy: one source feeding two consumers shares the *same* frame buffer,
//! rather than deep-copying it per edge. Each consumer reports the address of the pixel buffer
//! it received; the addresses must match.

use octans_core::*;
use octans_macros::node;
use octans_nodes::*;

struct FramePtr;

#[node(id = "test.frame_ptr", out = "ptr")]
impl FramePtr {
    fn process(&self, frame: &Image) -> u64 {
        frame.px.as_ptr() as u64
    }
}

#[test]
fn fanout_shares_one_frame_buffer() {
    let mut reg = Registry::new();
    register_primitives(&mut reg);
    register_node_types(&mut reg);
    let mut g = Graph::new(reg);

    let cam = g.add(SyntheticCamera { w: 64, h: 64, blobs: vec![(20, 20, 5)] });
    let a = g.add(FramePtr);
    let b = g.add(FramePtr);
    g.connect(cam, "frame", a, "frame").unwrap();
    g.connect(cam, "frame", b, "frame").unwrap();

    let engine = Mira::compile(&g).unwrap();
    let tick = engine.run_tick(&g);

    let pa = tick.output(a, "ptr").and_then(|v| v.downcast_ref::<u64>()).copied().unwrap();
    let pb = tick.output(b, "ptr").and_then(|v| v.downcast_ref::<u64>()).copied().unwrap();

    assert_ne!(pa, 0);
    assert_eq!(
        pa, pb,
        "both consumers must receive the SAME frame buffer — fan-out shares (Arc), not deep-copies"
    );
}
