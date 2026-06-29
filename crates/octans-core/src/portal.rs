//! Portals — temporal feedback edges (z⁻¹).
//!
//! A portal lets a value computed *downstream* feed back to an input *upstream* without
//! creating a dataflow cycle: a [`PortalWrite`] stores a value during a tick, and a
//! [`PortalRead`] yields the value written on the *previous* tick. Double-buffered and swapped
//! at each tick boundary, so a read never observes the current tick's write — the scheduler
//! still sees a DAG.
//!
//! Portals are for **cross-node temporal feedback** (a downstream result driving an upstream
//! input next tick). Node-*local* state (accumulators, controller memory) belongs in a node's
//! `Local` instead.

use crate::context::Context;
use crate::node::{Inputs, Node, Outputs, PortSpec};
use crate::value::{TypeSpec, Value};
use std::any::Any;
use std::mem;
use std::sync::{Arc, Mutex};

struct Slots {
    front: Value, // what a read sees this tick (written last tick)
    back: Value,  // what a write stores this tick (read next tick)
}

/// A cloneable handle to one temporal feedback slot. Clone to make matching reader/writer ends.
#[derive(Clone)]
pub struct Portal {
    ty: TypeSpec,
    slots: Arc<Mutex<Slots>>,
}

impl Portal {
    pub fn new(ty: TypeSpec, initial: Value) -> Self {
        Self {
            ty,
            slots: Arc::new(Mutex::new(Slots {
                front: initial.clone(),
                back: initial,
            })),
        }
    }

    /// A source node yielding the previous tick's value on `out_port`.
    pub fn reader(&self, out_port: &'static str) -> PortalRead {
        PortalRead {
            portal: self.clone(),
            out_port,
        }
    }

    /// A sink node capturing `in_port` into the portal for the next tick.
    pub fn writer(&self, in_port: &'static str) -> PortalWrite {
        PortalWrite {
            portal: self.clone(),
            in_port,
        }
    }

    /// Promote this tick's write to be next tick's read. Called by the interpreter at the tick
    /// boundary.
    pub(crate) fn swap(&self) {
        let mut guard = self.slots.lock().unwrap();
        let s: &mut Slots = &mut guard; // one DerefMut, then disjoint field borrows
        mem::swap(&mut s.front, &mut s.back);
    }
}

/// The read end of a [`Portal`] — a source that emits last tick's value.
pub struct PortalRead {
    portal: Portal,
    out_port: &'static str,
}

impl Node for PortalRead {
    fn type_id(&self) -> &'static str {
        "octans.core.portal_read"
    }
    fn inputs(&self) -> Vec<PortSpec> {
        Vec::new()
    }
    fn outputs(&self) -> Vec<PortSpec> {
        vec![PortSpec::new(self.out_port, self.portal.ty.clone())]
    }
    fn process(
        &self,
        _ctx: &Context,
        _local: &mut dyn Any,
        _inputs: &Inputs,
        outputs: &mut Outputs,
    ) {
        let v = self.portal.slots.lock().unwrap().front.clone();
        outputs.set_value(self.out_port, v);
    }
}

/// The write end of a [`Portal`] — a sink that captures a value for next tick.
pub struct PortalWrite {
    portal: Portal,
    in_port: &'static str,
}

impl Node for PortalWrite {
    fn type_id(&self) -> &'static str {
        "octans.core.portal_write"
    }
    fn inputs(&self) -> Vec<PortSpec> {
        vec![PortSpec::new(self.in_port, self.portal.ty.clone())]
    }
    fn outputs(&self) -> Vec<PortSpec> {
        Vec::new()
    }
    fn process(
        &self,
        _ctx: &Context,
        _local: &mut dyn Any,
        inputs: &Inputs,
        _outputs: &mut Outputs,
    ) {
        let v = inputs.value(self.in_port).clone();
        self.portal.slots.lock().unwrap().back = v;
    }
}
