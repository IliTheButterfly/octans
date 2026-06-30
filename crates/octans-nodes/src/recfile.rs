//! File I/O nodes — record and replay heterogeneous, multi-channel data streams.
//!
//! A record file is **JSON Lines**: the first line is a header declaring the schema
//! (`channel name -> type id`), and each subsequent line is one tick's frame — a JSON object of
//! the channels present that tick. This makes a single file hold *multiple value types* at once
//! (e.g. an `Image` channel alongside a `u32` count), and makes record/replay streaming and
//! crash-tolerant.
//!
//! - [`Recorder`]: a sink. You declare typed channels and wire values in; each tick it appends a
//!   frame of whatever channels are present. Built for deterministic replay (e.g. capture camera
//!   frames + observations, then re-run without live hardware — great for benchmarking).
//! - [`Replayer`]: a source. [`Replayer::open`] reads the file header at construction and
//!   **populates its output ports from the file's own schema** — so the loader knows its types
//!   without the author restating them. It emits one frame per tick; past the end it emits
//!   nothing (downstream simply skips).
//!
//! (De)serialization is driven by the type registry's `Serializer`/`Deserializer` entries, so any
//! registered, serde-able type flows through unchanged.
//!
//! Fidelity: integer/byte channels (e.g. `Image`) round-trip bit-exactly. Floating-point channels
//! are preserved to full f64 precision but, because the format is JSON *text*, may differ by up to
//! one ULP on replay for some values — replay is numerically faithful, not always bit-identical.
//! A binary format (bit-exact floats) can be added later behind the same node API.

use octans_core::{
    Context, Deserializer, Inputs, LogLevel, Node, Outputs, PortSpec, RegisteredType, Registry,
    Serializer, TypeId, TypeSpec, Value,
};
use std::any::Any;
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

fn leak(s: &str) -> &'static str {
    Box::leak(s.to_string().into_boxed_str())
}

const FORMAT_TAG: &str = "octans_rec";

// ---------------------------------------------------------------------------
// Recorder
// ---------------------------------------------------------------------------

/// A sink that appends one frame per tick to a JSON-Lines record file. Declare channels with
/// [`channel`](Recorder::channel); each tick, every channel whose input is present is serialized
/// into that tick's frame. Channels are optional, so a frame holds exactly what was produced.
pub struct Recorder {
    path: PathBuf,
    channels: Vec<(&'static str, TypeId)>,
    sers: OnceLock<Vec<Option<Serializer>>>,
    writer: Mutex<Option<BufWriter<File>>>,
}

impl Recorder {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            channels: Vec::new(),
            sers: OnceLock::new(),
            writer: Mutex::new(None),
        }
    }

    /// Declare a typed channel/port named `name`.
    pub fn channel<T: RegisteredType>(mut self, name: &str) -> Self {
        self.channels.push((leak(name), T::ID));
        self
    }

    /// The header line: `{"octans_rec":1,"schema":[["name","type.id"],...]}`.
    fn header_json(&self) -> serde_json::Value {
        let schema: Vec<[&str; 2]> = self.channels.iter().map(|(n, id)| [*n, *id]).collect();
        serde_json::json!({ FORMAT_TAG: 1, "schema": schema })
    }
}

impl Node for Recorder {
    fn node_type(&self) -> &'static str {
        "octans.io.recorder"
    }
    fn inputs(&self) -> Vec<PortSpec> {
        self.channels
            .iter()
            .map(|(name, id)| PortSpec::new(name, TypeSpec::scalar(id)).optional())
            .collect()
    }
    fn outputs(&self) -> Vec<PortSpec> {
        Vec::new()
    }

    fn prepare(&self, registry: &Registry) {
        let sers = self
            .channels
            .iter()
            .map(|(_, id)| registry.serializer(id))
            .collect();
        let _ = self.sers.set(sers);
    }

    fn process(&self, ctx: &Context, _l: &mut dyn Any, inputs: &Inputs, _o: &mut Outputs) {
        let mut guard = match self.writer.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        // Lazily open the file and write the header on the first tick.
        if guard.is_none() {
            match File::create(&self.path) {
                Ok(f) => {
                    let mut w = BufWriter::new(f);
                    if writeln!(w, "{}", self.header_json()).is_err() {
                        ctx.log(LogLevel::Error, "recorder", "failed to write record header");
                        return;
                    }
                    *guard = Some(w);
                }
                Err(e) => {
                    ctx.log(
                        LogLevel::Error,
                        "recorder",
                        format!("cannot create {}: {e}", self.path.display()),
                    );
                    return;
                }
            }
        }

        let sers = self.sers.get();
        let mut frame = serde_json::Map::new();
        for (i, (name, _id)) in self.channels.iter().enumerate() {
            let Some(v) = inputs.get_value(name) else {
                continue; // channel not produced this tick
            };
            let ser = sers.and_then(|s| s.get(i).copied().flatten());
            match ser.and_then(|f| f(v)) {
                Some(j) => {
                    frame.insert((*name).to_string(), j);
                }
                None => ctx.log(
                    LogLevel::Warning,
                    "recorder",
                    format!("channel `{name}` has no serializer; skipped"),
                ),
            }
        }

        if let Some(w) = guard.as_mut() {
            let line = serde_json::Value::Object(frame);
            // Flush each tick so a concurrent/after-the-fact reader sees a consistent file.
            if writeln!(w, "{line}").and_then(|_| w.flush()).is_err() {
                ctx.log(LogLevel::Error, "recorder", "failed to write frame");
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Replayer
// ---------------------------------------------------------------------------

type Frame = HashMap<&'static str, Value>;

/// A source that emits one recorded frame per tick. Its output ports come from the file's own
/// header (read at [`open`](Replayer::open)), so the loader's types are populated from the file.
pub struct Replayer {
    path: PathBuf,
    schema: Vec<(&'static str, TypeId)>,
    frames: OnceLock<Vec<Frame>>,
    loop_frames: bool,
}

impl Replayer {
    /// Open a record file and read its header, populating this node's output ports from the
    /// file's schema. Frame data is loaded later at compile time (the registry's deserializers
    /// must be in scope then).
    pub fn open(path: impl Into<PathBuf>) -> std::io::Result<Self> {
        use std::io::{Error, ErrorKind};
        let path = path.into();
        let file = File::open(&path)?;
        let first = BufReader::new(file)
            .lines()
            .next()
            .ok_or_else(|| Error::new(ErrorKind::InvalidData, "empty record file"))??;
        let header: serde_json::Value = serde_json::from_str(&first)
            .map_err(|e| Error::new(ErrorKind::InvalidData, format!("bad header: {e}")))?;
        if header.get(FORMAT_TAG).is_none() {
            return Err(Error::new(
                ErrorKind::InvalidData,
                "not an octans record file",
            ));
        }
        let mut schema = Vec::new();
        if let Some(arr) = header.get("schema").and_then(|s| s.as_array()) {
            for entry in arr {
                if let (Some(name), Some(id)) = (
                    entry.get(0).and_then(|v| v.as_str()),
                    entry.get(1).and_then(|v| v.as_str()),
                ) {
                    schema.push((leak(name), leak(id) as TypeId));
                }
            }
        }
        Ok(Self {
            path,
            schema,
            frames: OnceLock::new(),
            loop_frames: false,
        })
    }

    /// Loop the recording: once past the last frame, wrap back to the first. Handy for feeding a
    /// finite recording into a long-running consumer — e.g. the autotuner, which needs many ticks
    /// to benchmark each strategy variant.
    pub fn looping(mut self) -> Self {
        self.loop_frames = true;
        self
    }

    /// Build a replayer with an **author-declared** schema instead of reading it from the file
    /// header. Use this when you want to assert the types a file loads, or to reference a file
    /// that doesn't exist yet at graph-build time (the frames are read at compile, by which point
    /// it must exist). Frame reading tolerates files with or without a header line.
    pub fn with_schema(path: impl Into<PathBuf>, schema: &[(&str, TypeId)]) -> Self {
        Self {
            path: path.into(),
            schema: schema.iter().map(|(n, id)| (leak(n), *id)).collect(),
            frames: OnceLock::new(),
            loop_frames: false,
        }
    }
}

/// True if a line is an octans record header (vs a data frame).
fn is_header_line(line: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(line)
        .ok()
        .and_then(|v| v.get(FORMAT_TAG).cloned())
        .is_some()
}

impl Node for Replayer {
    fn node_type(&self) -> &'static str {
        "octans.io.replayer"
    }
    fn inputs(&self) -> Vec<PortSpec> {
        Vec::new()
    }
    fn outputs(&self) -> Vec<PortSpec> {
        self.schema
            .iter()
            .map(|(name, id)| PortSpec::new(name, TypeSpec::scalar(id)))
            .collect()
    }

    fn prepare(&self, registry: &Registry) {
        // Map channel name -> (leaked name, deserializer) for this file's schema.
        let de_by_name: HashMap<&str, (&'static str, Option<Deserializer>)> = self
            .schema
            .iter()
            .map(|(name, id)| (*name, (*name, registry.deserializer(id))))
            .collect();

        let mut frames: Vec<Frame> = Vec::new();
        if let Ok(file) = File::open(&self.path) {
            for (i, line) in BufReader::new(file)
                .lines()
                .map_while(Result::ok)
                .enumerate()
            {
                if line.trim().is_empty() {
                    continue;
                }
                // Skip the header line if present (the first line), so both `open` and
                // `with_schema` read headered files correctly.
                if i == 0 && is_header_line(&line) {
                    continue;
                }
                let mut frame = Frame::new();
                if let Ok(serde_json::Value::Object(obj)) = serde_json::from_str(&line) {
                    for (name, jv) in obj {
                        if let Some((leaked, Some(de))) = de_by_name.get(name.as_str()) {
                            if let Some(v) = de(&jv) {
                                frame.insert(*leaked, v);
                            }
                        }
                    }
                }
                frames.push(frame);
            }
        }
        let _ = self.frames.set(frames);
    }

    fn process(&self, ctx: &Context, _l: &mut dyn Any, _i: &Inputs, outputs: &mut Outputs) {
        let Some(frames) = self.frames.get() else {
            return;
        };
        if frames.is_empty() {
            return;
        }
        // Ticks start at 1; frame 0 is the first recorded tick. Past the end we emit nothing
        // (downstream skips), unless looping — then we wrap back to the start.
        let raw = (ctx.tick() as usize).saturating_sub(1);
        let idx = if self.loop_frames {
            raw % frames.len()
        } else {
            raw
        };
        if let Some(frame) = frames.get(idx) {
            for (name, v) in frame {
                outputs.set_value(name, v.clone());
            }
        }
    }
}
