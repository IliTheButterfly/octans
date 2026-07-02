//! Undo/redo — a command stack of invertible edits.
//!
//! `Graph` can't be cloned (boxed nodes) and `GraphSpec` doesn't cover closure-carrying nodes, so
//! snapshots are out. Instead every edit records enough to invert itself, and because deletion
//! tombstones (slots are never reused), undo/redo can restore a node *into its original `NodeId`*
//! via `replace_node` — edges and ids stay consistent by construction.

use crate::OctansApp;
use eframe::egui;
use octans_core::{Node, NodeId};
use serde_json::Value;

/// `(from, from_port, to, to_port)` — an edge as owned data.
pub(crate) type EdgeRec = (usize, String, usize, String);

/// One invertible edit. Stored on the undo stack; `apply_edit(_, true)` re-does it,
/// `apply_edit(_, false)` un-does it.
pub enum EditAction {
    /// A node appended from the palette (undo tombstones the slot; redo rebuilds into it).
    AddNode {
        id: usize,
        type_id: String,
        config: Value,
    },
    /// A node removed (undo rebuilds it in place and restores its edges + position).
    DeleteNode {
        id: usize,
        type_id: String,
        config: Value,
        edges: Vec<EdgeRec>,
        pos: Option<(f32, f32)>,
    },
    /// A wire created into `(to, to_port)`, possibly replacing previous feeders.
    Connect {
        from: usize,
        from_port: String,
        to: usize,
        to_port: String,
        replaced: Vec<(usize, String)>,
    },
    /// The feeders of `(to, to_port)` removed.
    Disconnect {
        to: usize,
        to_port: String,
        removed: Vec<(usize, String)>,
    },
    /// A node's config changed (consecutive edits to the same node coalesce).
    ParamEdit {
        id: usize,
        type_id: String,
        before: Value,
        after: Value,
    },
    /// A node dragged to a new position (`before: None` = it was auto-laid).
    MoveNode {
        id: usize,
        before: Option<(f32, f32)>,
        after: (f32, f32),
    },
}

impl OctansApp {
    /// Rebuild a node instance for undo/redo: prefer the serde factory (exact config), fall back
    /// to the catalog constructor (default config — best effort for non-serde types; a port
    /// mismatch surfaces as unrestored edges + a compile error, never corruption).
    pub(crate) fn rebuild_node(&self, type_id: &str, config: &Value) -> Option<Box<dyn Node>> {
        if !config.is_null() {
            if let Some(n) = self.node_registry.build(type_id, config) {
                return Some(n);
            }
        }
        self.catalog.make(type_id)
    }

    /// Record a fresh edit: pushes onto the undo stack and invalidates the redo stack.
    pub(crate) fn push_edit(&mut self, a: EditAction) {
        self.undo_stack.push(a);
        if self.undo_stack.len() > 200 {
            self.undo_stack.remove(0);
        }
        self.redo_stack.clear();
    }

    pub fn undo(&mut self) {
        if let Some(a) = self.undo_stack.pop() {
            self.apply_edit(&a, false);
            self.redo_stack.push(a);
        }
    }

    pub fn redo(&mut self) {
        if let Some(a) = self.redo_stack.pop() {
            self.apply_edit(&a, true);
            self.undo_stack.push(a);
        }
    }

    fn apply_edit(&mut self, a: &EditAction, forward: bool) {
        match a {
            EditAction::AddNode {
                id,
                type_id,
                config,
            } => {
                if forward {
                    if let Some(node) = self.rebuild_node(type_id, config) {
                        self.graph.replace_node(NodeId(*id), node);
                    }
                } else {
                    self.graph.remove_node(NodeId(*id));
                    self.manual_pos.remove(id);
                }
            }
            EditAction::DeleteNode {
                id,
                type_id,
                config,
                edges,
                pos,
            } => {
                if forward {
                    self.graph.remove_node(NodeId(*id));
                    self.manual_pos.remove(id);
                } else if let Some(node) = self.rebuild_node(type_id, config) {
                    self.graph.replace_node(NodeId(*id), node);
                    for (f, fp, t, tp) in edges {
                        let _ = self.graph.connect(NodeId(*f), fp, NodeId(*t), tp);
                    }
                    if let Some((x, y)) = pos {
                        self.manual_pos.insert(*id, egui::pos2(*x, *y));
                    }
                }
            }
            EditAction::Connect {
                from,
                from_port,
                to,
                to_port,
                replaced,
            } => {
                self.graph.disconnect_input(NodeId(*to), to_port);
                if forward {
                    let _ = self
                        .graph
                        .connect(NodeId(*from), from_port, NodeId(*to), to_port);
                } else {
                    for (f, fp) in replaced {
                        let _ = self.graph.connect(NodeId(*f), fp, NodeId(*to), to_port);
                    }
                }
            }
            EditAction::Disconnect {
                to,
                to_port,
                removed,
            } => {
                if forward {
                    self.graph.disconnect_input(NodeId(*to), to_port);
                } else {
                    for (f, fp) in removed {
                        let _ = self.graph.connect(NodeId(*f), fp, NodeId(*to), to_port);
                    }
                }
            }
            EditAction::ParamEdit {
                id,
                type_id,
                before,
                after,
            } => {
                let cfg = if forward { after } else { before };
                if let Some(node) = self.node_registry.build(type_id, cfg) {
                    self.graph.replace_node(NodeId(*id), node);
                }
            }
            EditAction::MoveNode { id, before, after } => {
                if forward {
                    self.manual_pos.insert(*id, egui::pos2(after.0, after.1));
                } else {
                    match before {
                        Some((x, y)) => {
                            self.manual_pos.insert(*id, egui::pos2(*x, *y));
                        }
                        None => {
                            self.manual_pos.remove(id);
                        }
                    }
                }
            }
        }
        self.rebuild_after_edit();
    }

    /// Add a node of the given catalog type (the palette's ➕), recording the edit.
    pub fn add_node_from_catalog(&mut self, type_id: &str) -> Option<NodeId> {
        let node = self.catalog.make(type_id)?;
        let config = node.to_json();
        let id = self.graph.add_boxed(node);
        self.push_edit(EditAction::AddNode {
            id: id.0,
            type_id: type_id.to_string(),
            config,
        });
        self.rebuild_after_edit();
        Some(id)
    }

    /// Add a node built from a serde factory with an explicit config (the palette's structural
    /// pickers — e.g. a Gather with a chosen element type and arity), recording the edit.
    pub fn add_node_with_config(
        &mut self,
        type_id: &str,
        config: serde_json::Value,
    ) -> Option<NodeId> {
        let node = self.node_registry.build(type_id, &config)?;
        let id = self.graph.add_boxed(node);
        self.push_edit(EditAction::AddNode {
            id: id.0,
            type_id: type_id.to_string(),
            config,
        });
        self.rebuild_after_edit();
        Some(id)
    }

    /// Disconnect every feeder of an input port, recording the edit (the canvas's right-click).
    pub fn disconnect_edit(&mut self, to: NodeId, to_port: &str) {
        let removed: Vec<(usize, String)> = self
            .graph
            .edges()
            .filter(|e| e.to == to && e.to_port == to_port)
            .map(|e| (e.from.0, e.from_port.to_string()))
            .collect();
        if removed.is_empty() {
            return;
        }
        self.graph.disconnect_input(to, to_port);
        self.push_edit(EditAction::Disconnect {
            to: to.0,
            to_port: to_port.to_string(),
            removed,
        });
        self.rebuild_after_edit();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::SceneKind;
    use octans_core::TOMBSTONE_TYPE;

    #[test]
    fn add_undo_redo_round_trips() {
        let mut app = OctansApp::from_scene(SceneKind::Diagnostics);
        let n0 = app.graph.node_count();
        let id = app
            .add_node_from_catalog("octans.std.synthetic_camera")
            .unwrap();
        assert_eq!(app.graph.node_count(), n0 + 1);

        app.undo();
        assert_eq!(
            app.graph.node(id).unwrap().node_type(),
            TOMBSTONE_TYPE,
            "undo tombstones the added node"
        );
        app.redo();
        assert_eq!(
            app.graph.node(id).unwrap().node_type(),
            "octans.std.synthetic_camera",
            "redo rebuilds it into the same slot"
        );
    }

    #[test]
    fn connect_and_disconnect_undo() {
        let mut app = OctansApp::from_scene(SceneKind::Diagnostics);
        let cam = app
            .add_node_from_catalog("octans.std.synthetic_camera")
            .unwrap();
        let thr = app.add_node_from_catalog("octans.std.threshold").unwrap();

        assert!(app.try_connect(cam, "frame", thr, "image"));
        let edges_after = app.graph.edges().count();
        app.undo(); // undo the connect
        assert_eq!(app.graph.edges().count(), edges_after - 1);
        app.redo();
        assert_eq!(app.graph.edges().count(), edges_after);

        app.disconnect_edit(thr, "image");
        assert_eq!(app.graph.edges().count(), edges_after - 1);
        app.undo(); // undo the disconnect → wire restored
        assert_eq!(app.graph.edges().count(), edges_after);
    }

    #[test]
    fn delete_undo_restores_node_and_edges() {
        let mut app = OctansApp::from_scene(SceneKind::Diagnostics);
        let thr = NodeId(1); // camera(0) → threshold(1) → blobcount(2) → …
        assert_eq!(
            app.graph.node(thr).unwrap().node_type(),
            "octans.std.threshold"
        );
        let edges_before = app.graph.edges().count();

        app.delete_node(thr);
        assert_eq!(app.graph.node(thr).unwrap().node_type(), TOMBSTONE_TYPE);
        assert!(app.compile_error.is_some(), "blobcount lost its feeder");

        app.undo();
        assert_eq!(
            app.graph.node(thr).unwrap().node_type(),
            "octans.std.threshold"
        );
        assert_eq!(app.graph.edges().count(), edges_before, "edges restored");
        assert!(app.compile_error.is_none(), "graph compiles again");
    }

    #[test]
    fn param_edit_undo_restores_config() {
        let mut app = OctansApp::from_scene(SceneKind::Diagnostics);
        let cam = NodeId(0);
        let before = app.graph.node(cam).unwrap().to_json();
        let mut after = before.clone();
        after["w"] = serde_json::json!(128);

        let node = app
            .node_registry
            .build("octans.std.synthetic_camera", &after)
            .unwrap();
        app.graph.replace_node(cam, node);
        app.push_edit(EditAction::ParamEdit {
            id: cam.0,
            type_id: "octans.std.synthetic_camera".into(),
            before: before.clone(),
            after,
        });

        assert_eq!(app.graph.node(cam).unwrap().to_json()["w"], 128);
        app.undo();
        assert_eq!(app.graph.node(cam).unwrap().to_json(), before);
    }
}
