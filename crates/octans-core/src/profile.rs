//! Always-on per-node profiling — the **benchmarkable** pillar.
//!
//! Every tick, the engine records how long each node's `process` took. This is the *speed*
//! signal the autotuner optimizes against (the *accuracy* signal is author-defined, via a
//! score-sink node). Measuring it is universal and free — so it's on by default, not an opt-in
//! helper that never gets wired up (the recurring fate in every prior attempt).

use crate::graph::NodeId;
use std::time::Duration;

/// Rolling latency stats for one node instance.
#[derive(Default, Clone, Debug)]
pub struct NodeStat {
    pub samples: u64,
    pub total: Duration,
    pub last: Duration,
    pub max: Duration,
}

impl NodeStat {
    pub fn mean(&self) -> Duration {
        if self.samples == 0 {
            Duration::ZERO
        } else {
            self.total / self.samples as u32
        }
    }
}

/// Per-node latency profile, indexed by [`NodeId`].
#[derive(Default, Clone, Debug)]
pub struct Profile {
    stats: Vec<NodeStat>,
}

impl Profile {
    pub(crate) fn with_len(n: usize) -> Self {
        Self {
            stats: vec![NodeStat::default(); n],
        }
    }

    pub(crate) fn record(&mut self, node: usize, dur: Duration) {
        let s = &mut self.stats[node];
        s.samples += 1;
        s.total += dur;
        s.last = dur;
        if dur > s.max {
            s.max = dur;
        }
    }

    /// Stats for a node.
    pub fn node(&self, node: NodeId) -> &NodeStat {
        &self.stats[node.0]
    }

    /// Iterate `(node, stats)` for every node.
    pub fn iter(&self) -> impl Iterator<Item = (NodeId, &NodeStat)> {
        self.stats.iter().enumerate().map(|(i, s)| (NodeId(i), s))
    }

    pub fn len(&self) -> usize {
        self.stats.len()
    }

    pub fn is_empty(&self) -> bool {
        self.stats.is_empty()
    }
}
