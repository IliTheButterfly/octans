//! Diagnostics nodes — the engine's observability layer.
//!
//! [`Log`] turns a value into a severity-tagged [`Diagnostic`](octans_core::Diagnostic) message
//! each tick; [`Probe`] is a transparent tap you drop on any edge to record the value flowing
//! through it (and pass it along unchanged). Both emit onto the [`Tick`](octans_core::Tick)'s
//! `diagnostics`, so they're inspectable headlessly today and feed a log view once there's a UI.
//!
//! Both are generic over the value type and require `Debug` (to render the message), so they're
//! hand-written `Node` impls rather than `#[node]` (which doesn't do generics yet).

use octans_core::{
    Context, Inputs, LogLevel, Node, Outputs, PortSpec, RegisteredType, TypeSpec, Value,
};
use std::any::Any;
use std::fmt::Debug;
use std::marker::PhantomData;

fn leak(s: &str) -> &'static str {
    Box::leak(s.to_string().into_boxed_str())
}

/// A sink that logs its input value at a fixed severity each tick, tagged with `source`.
///
/// Wire any value into `value`; every tick it produces a `Debug`-rendered
/// [`Diagnostic`](octans_core::Diagnostic) at `level`. As a sink it has no output; if its input
/// is absent on a tick (the upstream skipped) it simply logs nothing that tick.
pub struct Log<T> {
    level: LogLevel,
    source: String,
    _pd: PhantomData<fn() -> T>,
}

impl<T: RegisteredType + Debug> Log<T> {
    pub fn new(level: LogLevel, source: impl Into<String>) -> Self {
        Self {
            level,
            source: source.into(),
            _pd: PhantomData,
        }
    }

    /// Shorthand constructors.
    pub fn error(source: impl Into<String>) -> Self {
        Self::new(LogLevel::Error, source)
    }
    pub fn warning(source: impl Into<String>) -> Self {
        Self::new(LogLevel::Warning, source)
    }
    pub fn info(source: impl Into<String>) -> Self {
        Self::new(LogLevel::Info, source)
    }
}

impl<T: RegisteredType + Debug> Node for Log<T> {
    fn node_type(&self) -> &'static str {
        "octans.diag.log"
    }
    fn inputs(&self) -> Vec<PortSpec> {
        vec![PortSpec::new("value", T::type_spec())]
    }
    fn outputs(&self) -> Vec<PortSpec> {
        Vec::new()
    }
    fn process(&self, ctx: &Context, _l: &mut dyn Any, inputs: &Inputs, _o: &mut Outputs) {
        let v = inputs.get::<T>("value");
        ctx.log(self.level, self.source.clone(), format!("{v:?}"));
    }
}

/// A transparent tap: passes `in` straight through to `out` unchanged, and records the value it
/// saw as an [`Info`](octans_core::LogLevel::Info) (configurable) diagnostic labelled `label`.
///
/// Drop a `Probe` on any edge to inspect a graph mid-pipeline without rewiring; remove it and the
/// dataflow is identical. Because it's a pass-through, downstream nodes still receive the value.
pub struct Probe<T> {
    label: String,
    level: LogLevel,
    _pd: PhantomData<fn() -> T>,
}

impl<T: RegisteredType + Clone + Debug> Probe<T> {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            level: LogLevel::Info,
            _pd: PhantomData,
        }
    }

    /// Record at a specific severity (default is `Info`).
    pub fn at(mut self, level: LogLevel) -> Self {
        self.level = level;
        self
    }
}

impl<T: RegisteredType + Clone + Debug> Node for Probe<T> {
    fn node_type(&self) -> &'static str {
        "octans.diag.probe"
    }
    fn inputs(&self) -> Vec<PortSpec> {
        vec![PortSpec::new("in", T::type_spec())]
    }
    fn outputs(&self) -> Vec<PortSpec> {
        vec![PortSpec::new("out", T::type_spec())]
    }
    fn process(&self, ctx: &Context, _l: &mut dyn Any, inputs: &Inputs, outputs: &mut Outputs) {
        let v = inputs.get::<T>("in");
        ctx.log(self.level, self.label.clone(), format!("{v:?}"));
        outputs.set("out", v.clone());
    }
}

/// Renders a type-erased value of type `T` via its `Debug` impl (for [`LogFmt`] args).
fn render_dbg<T: Any + Debug>(v: &Value) -> String {
    v.downcast_ref::<T>()
        .map(|x| format!("{x:?}"))
        .unwrap_or_default()
}

type Render = fn(&Value) -> String;

/// A logger driven by a **format string** with `{{name}}` placeholders. Each placeholder is
/// filled by the value on the like-named typed input port. Declare ports (and their types) with
/// [`arg`](LogFmt::arg):
///
/// ```ignore
/// let l = LogFmt::warning("vision", "found {{n}} blobs near {{p}}")
///     .arg::<u32>("n")
///     .arg::<Pt3>("p");
/// ```
///
/// Each tick it renders every arg (via `Debug`), substitutes them into the template, and emits a
/// [`Diagnostic`](octans_core::Diagnostic) at the chosen severity. Args are required inputs, so if
/// any is absent the node skips that tick (it never logs a half-filled template).
pub struct LogFmt {
    level: LogLevel,
    source: String,
    template: String,
    args: Vec<(&'static str, TypeSpec, Render)>,
}

impl LogFmt {
    pub fn new(level: LogLevel, source: impl Into<String>, template: impl Into<String>) -> Self {
        Self {
            level,
            source: source.into(),
            template: template.into(),
            args: Vec::new(),
        }
    }
    pub fn error(source: impl Into<String>, template: impl Into<String>) -> Self {
        Self::new(LogLevel::Error, source, template)
    }
    pub fn warning(source: impl Into<String>, template: impl Into<String>) -> Self {
        Self::new(LogLevel::Warning, source, template)
    }
    pub fn info(source: impl Into<String>, template: impl Into<String>) -> Self {
        Self::new(LogLevel::Info, source, template)
    }

    /// Declare a typed input port named `name`; its value fills `{{name}}` in the template.
    pub fn arg<T: RegisteredType + Debug>(mut self, name: &str) -> Self {
        self.args
            .push((leak(name), T::type_spec(), render_dbg::<T>));
        self
    }
}

impl Node for LogFmt {
    fn node_type(&self) -> &'static str {
        "octans.diag.log_fmt"
    }
    fn inputs(&self) -> Vec<PortSpec> {
        self.args
            .iter()
            .map(|(name, ty, _)| PortSpec::new(name, ty.clone()))
            .collect()
    }
    fn outputs(&self) -> Vec<PortSpec> {
        Vec::new()
    }
    fn process(&self, ctx: &Context, _l: &mut dyn Any, inputs: &Inputs, _o: &mut Outputs) {
        let mut msg = self.template.clone();
        for (name, _ty, render) in &self.args {
            if let Some(v) = inputs.get_value(name) {
                let pat = ["{{", name, "}}"].concat();
                msg = msg.replace(&pat, &render(v));
            }
        }
        ctx.log(self.level, self.source.clone(), msg);
    }
}
