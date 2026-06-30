//! The global context — shared, read-mostly state available to every node each tick.
//!
//! Holds the tick (frame) counter and a type-keyed **resource map**: data loaded once and
//! shared read-only by all nodes (calibration tables, a GPU device handle, config). Access is
//! `&Context`, so reads parallelize freely. Shared *mutable* global state, if ever needed, is
//! an explicitly-synchronized resource a node opts into — never implicit.

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::Mutex;

/// Severity of a [`Diagnostic`] emitted by a node (e.g. the `Log`/`Probe` nodes).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogLevel {
    Error,
    Warning,
    Info,
}

impl std::fmt::Display for LogLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            LogLevel::Error => "ERROR",
            LogLevel::Warning => "WARN",
            LogLevel::Info => "INFO",
        })
    }
}

/// A diagnostic message a node emitted during a tick — collected on the [`Tick`](crate::Tick)
/// (like faults/skips) so it can be inspected headlessly now and rendered in a log view later.
#[derive(Clone, Debug)]
pub struct Diagnostic {
    pub level: LogLevel,
    /// A human label identifying who emitted it (a node's configured source/probe label).
    pub source: String,
    pub message: String,
    /// The tick this was emitted on.
    pub tick: u64,
}

#[derive(Default)]
pub struct Context {
    tick: u64,
    resources: HashMap<TypeId, Box<dyn Any + Send + Sync>>,
    /// Per-tick diagnostic sink. Nodes push via `&Context` (so it's behind a `Mutex` — writes
    /// happen concurrently across a level), and the interpreter drains it into each `Tick`.
    diagnostics: Mutex<Vec<Diagnostic>>,
}

impl Context {
    pub fn new() -> Self {
        Self::default()
    }

    /// Monotonic tick/frame counter. Starts at 0; the interpreter advances it each tick (so the
    /// first tick a node sees is 1).
    pub fn tick(&self) -> u64 {
        self.tick
    }

    pub(crate) fn advance(&mut self) {
        self.tick += 1;
    }

    /// Insert a shared resource (loaded once, read by nodes via [`resource`](Self::resource)).
    pub fn insert_resource<T: Any + Send + Sync>(&mut self, value: T) {
        self.resources.insert(TypeId::of::<T>(), Box::new(value));
    }

    /// Borrow a shared resource by type, if present.
    pub fn resource<T: Any + Send + Sync>(&self) -> Option<&T> {
        self.resources
            .get(&TypeId::of::<T>())
            .and_then(|b| b.downcast_ref::<T>())
    }

    /// Emit a diagnostic from a node (available via `&Context`, e.g. a `#[ctx]` param). It is
    /// stamped with the current tick and collected onto this tick's [`Tick`](crate::Tick).
    pub fn log(&self, level: LogLevel, source: impl Into<String>, message: impl Into<String>) {
        let d = Diagnostic {
            level,
            source: source.into(),
            message: message.into(),
            tick: self.tick,
        };
        if let Ok(mut buf) = self.diagnostics.lock() {
            buf.push(d);
        }
    }

    /// Drain the diagnostics accumulated this tick (called by the interpreter at tick end).
    pub(crate) fn take_diagnostics(&self) -> Vec<Diagnostic> {
        self.diagnostics
            .lock()
            .map(|mut b| std::mem::take(&mut *b))
            .unwrap_or_default()
    }
}
