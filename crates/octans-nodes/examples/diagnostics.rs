//! Diagnostics demo: a Probe taps an edge mid-pipeline and a Log reports the result, each
//! emitting onto the tick's `diagnostics` stream. Run: `cargo run -p octans-nodes --example diagnostics`

use octans_core::*;
use octans_nodes::*;

fn main() {
    let mut reg = Registry::new();
    register_primitives(&mut reg);
    register_node_types(&mut reg);
    let mut g = Graph::new(reg);

    // camera -> threshold -> [probe the mask's blob count] -> log it
    let cam = g.add(SyntheticCamera {
        w: 64,
        h: 64,
        blobs: vec![(16, 16, 5), (40, 40, 7)],
    });
    let thr = g.add(Threshold);
    let blobs = g.add(BlobCount);
    let probe = g.add(Probe::<u32>::new("blob-count"));
    let logger = g.add(Log::<u32>::warning("vision"));
    // A format-string logger: {{count}} is filled from the like-named typed input.
    let report = g.add(LogFmt::info("vision", "frame had {{count}} blobs").arg::<u32>("count"));

    g.connect(cam, "frame", thr, "image").unwrap();
    g.connect(thr, "mask", blobs, "mask").unwrap();
    g.connect(blobs, "count", probe, "in").unwrap();
    g.connect(probe, "out", logger, "value").unwrap();
    g.connect(probe, "out", report, "count").unwrap();

    let mut engine = Mira::compile(&g).unwrap();
    for _ in 0..3 {
        let tick = engine.run_tick(&g);
        println!(
            "--- tick {} ---",
            tick.diagnostics.first().map(|d| d.tick).unwrap_or(0)
        );
        for d in &tick.diagnostics {
            println!("  [{}] {}: {}", d.level, d.source, d.message);
        }
    }
}
