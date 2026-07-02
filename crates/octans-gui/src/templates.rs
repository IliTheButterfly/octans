//! Group templates in the editor — Blender-style structural editing.
//!
//! Shift-click nodes to build a selection, **capture** it as a data-defined template
//! ([`BodySpec`]): internal edges stay inside, edges crossing the selection boundary (plus
//! unconnected required inputs) become the template's boundary ports. Templates then instantiate
//! from the palette — inlined as a group, fanned out under **Map** (data-parallel), or iterated
//! under **Loop** (a for/repeat zone). Templates persist in the save file.

use crate::history::{EdgeRec, EditAction};
use crate::OctansApp;
use eframe::egui;
use octans_core::{
    BodySpec, BoundarySpec, EdgeSpec, GroupTemplate, Loop, Map, Node, NodeId, NodeSpec,
};

impl OctansApp {
    /// Capture the current selection as a named template. Fails (with a toolbar message) if a
    /// selected node's type has no serde factory — templates must be data all the way down.
    pub fn capture_selection(&mut self) {
        let mut sel: Vec<usize> = self.sel_set.iter().copied().collect();
        sel.sort_unstable();
        if sel.is_empty() {
            self.edit_error = Some("nothing selected — shift-click nodes first".into());
            return;
        }
        let local: std::collections::HashMap<usize, usize> =
            sel.iter().enumerate().map(|(l, &g)| (g, l)).collect();

        let mut nodes = Vec::new();
        for &gid in &sel {
            let Some(n) = self.graph.node(NodeId(gid)) else {
                continue;
            };
            nodes.push(NodeSpec {
                type_id: n.node_type().to_string(),
                config: n.to_json(),
            });
        }

        let all_edges: Vec<_> = self.graph.edges().collect();
        let mut edges = Vec::new();
        let mut inputs: Vec<BoundarySpec> = Vec::new();
        let mut outputs: Vec<BoundarySpec> = Vec::new();
        for e in &all_edges {
            match (local.get(&e.from.0), local.get(&e.to.0)) {
                // fully inside → internal edge
                (Some(&lf), Some(&lt)) => edges.push(EdgeSpec {
                    from: lf,
                    from_port: e.from_port.to_string(),
                    to: lt,
                    to_port: e.to_port.to_string(),
                }),
                // fed from outside → boundary input
                (None, Some(&lt)) => {
                    if !inputs.iter().any(|b| b.node == lt && b.port == e.to_port) {
                        inputs.push(BoundarySpec {
                            name: format!("{}_{lt}", e.to_port),
                            node: lt,
                            port: e.to_port.to_string(),
                        });
                    }
                }
                // consumed outside → boundary output
                (Some(&lf), None) => {
                    if !outputs
                        .iter()
                        .any(|b| b.node == lf && b.port == e.from_port)
                    {
                        outputs.push(BoundarySpec {
                            name: format!("{}_{lf}", e.from_port),
                            node: lf,
                            port: e.from_port.to_string(),
                        });
                    }
                }
                _ => {}
            }
        }
        // Unconnected required inputs and unconsumed outputs also become boundary ports (so the
        // template is wireable even when captured from an island).
        for &gid in &sel {
            let l = local[&gid];
            let Some(n) = self.graph.node(NodeId(gid)) else {
                continue;
            };
            for p in n.inputs() {
                let fed = all_edges
                    .iter()
                    .any(|e| e.to.0 == gid && e.to_port == p.name);
                let listed = inputs.iter().any(|b| b.node == l && b.port == p.name);
                if !fed && !listed && p.default.is_none() && !p.optional {
                    inputs.push(BoundarySpec {
                        name: format!("{}_{l}", p.name),
                        node: l,
                        port: p.name.to_string(),
                    });
                }
            }
            for p in n.outputs() {
                let consumed = all_edges
                    .iter()
                    .any(|e| e.from.0 == gid && e.from_port == p.name);
                let listed = outputs.iter().any(|b| b.node == l && b.port == p.name);
                if !consumed && !listed {
                    outputs.push(BoundarySpec {
                        name: format!("{}_{l}", p.name),
                        node: l,
                        port: p.name.to_string(),
                    });
                }
            }
        }

        let spec = BodySpec {
            nodes,
            edges,
            inputs,
            outputs,
        };
        self.template_counter += 1;
        let name = format!("group{}", self.template_counter);
        match GroupTemplate::from_spec(&name, spec.clone(), self.node_registry.clone()) {
            Ok(_) => {
                self.templates.push((name.clone(), spec));
                self.edit_error = None;
                self.io_status = Some(format!("captured {} nodes as `{name}`", sel.len()));
            }
            Err(e) => self.edit_error = Some(format!("can't template: {e:?}")),
        }
    }

    /// Inline an instance of template `idx` into the graph (recorded as one undoable edit).
    pub fn instantiate_template(&mut self, idx: usize) {
        let Some((name, spec)) = self.templates.get(idx).cloned() else {
            return;
        };
        let tpl = match GroupTemplate::from_spec(&name, spec, self.node_registry.clone()) {
            Ok(t) => t,
            Err(e) => {
                self.edit_error = Some(format!("{e:?}"));
                return;
            }
        };
        let nodes_before = self.graph.node_count();
        let edges_before = self.graph.edges().count();
        match self.graph.add_group(&tpl) {
            Ok(_inst) => {
                let nodes: Vec<(usize, String, serde_json::Value)> = (nodes_before
                    ..self.graph.node_count())
                    .filter_map(|i| {
                        self.graph
                            .node(NodeId(i))
                            .map(|n| (i, n.node_type().to_string(), n.to_json()))
                    })
                    .collect();
                let edges: Vec<EdgeRec> = self
                    .graph
                    .edges()
                    .skip(edges_before)
                    .map(|e| {
                        (
                            e.from.0,
                            e.from_port.to_string(),
                            e.to.0,
                            e.to_port.to_string(),
                        )
                    })
                    .collect();
                self.push_edit(EditAction::AddMany { nodes, edges });
                self.rebuild_after_edit();
            }
            Err(e) => self.edit_error = Some(format!("{e:?}")),
        }
    }

    /// Build a Map/Loop node over a named template (used by placement and by redo).
    pub(crate) fn build_composite(
        &self,
        kind: &str,
        template: &str,
        count: usize,
    ) -> Option<Box<dyn Node>> {
        let (name, spec) = self.templates.iter().find(|(n, _)| n == template)?;
        let tpl = GroupTemplate::from_spec(name, spec.clone(), self.node_registry.clone()).ok()?;
        match kind {
            "map" => Some(Box::new(Map::group(&tpl))),
            "loop" => Some(Box::new(Loop::group(count, &tpl))),
            _ => None,
        }
    }

    /// Place a Map or Loop over template `idx` as a single node (recorded, undoable).
    pub fn add_composite(&mut self, kind: &str, idx: usize) {
        let Some((name, spec)) = self.templates.get(idx).cloned() else {
            return;
        };
        // Loop needs exactly one input and one output boundary (it feeds the output back).
        if kind == "loop" && (spec.inputs.len() != 1 || spec.outputs.len() != 1) {
            self.edit_error = Some(format!(
                "loop needs a 1-in/1-out template (this one is {}-in/{}-out)",
                spec.inputs.len(),
                spec.outputs.len()
            ));
            return;
        }
        let Some(node) = self.build_composite(kind, &name, self.loop_count) else {
            self.edit_error = Some("template no longer lowers".into());
            return;
        };
        let id = self.graph.add_boxed(node);
        self.push_edit(EditAction::AddComposite {
            id: id.0,
            kind: kind.to_string(),
            template: name,
            count: self.loop_count,
        });
        self.rebuild_after_edit();
    }

    /// The palette's "templates" section.
    pub(crate) fn templates_section(&mut self, ui: &mut egui::Ui) {
        ui.strong("templates");
        let sel_n = self.sel_set.len();
        let mut capture = false;
        let mut action: Option<(&'static str, usize)> = None;

        if sel_n > 0 {
            if ui
                .button(format!("⬛ capture {sel_n} selected as template"))
                .clicked()
            {
                capture = true;
            }
        } else {
            ui.weak("shift-click nodes, then capture them as a template");
        }
        ui.horizontal(|ui| {
            ui.label("loop count");
            ui.add(egui::DragValue::new(&mut self.loop_count).range(1..=64));
        });
        for (i, (name, spec)) in self.templates.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.strong(name);
                ui.weak(format!(
                    "{}n {}in {}out",
                    spec.nodes.len(),
                    spec.inputs.len(),
                    spec.outputs.len()
                ));
                if ui
                    .small_button("➕")
                    .on_hover_text("inline an instance")
                    .clicked()
                {
                    action = Some(("group", i));
                }
                if ui
                    .small_button("∥ map")
                    .on_hover_text("data-parallel fan-out over vectors")
                    .clicked()
                {
                    action = Some(("map", i));
                }
                if ui
                    .small_button("🔁 loop")
                    .on_hover_text("apply N times (repeat zone)")
                    .clicked()
                {
                    action = Some(("loop", i));
                }
            });
        }

        if capture {
            self.capture_selection();
        }
        match action {
            Some(("group", i)) => self.instantiate_template(i),
            Some((kind, i)) => self.add_composite(kind, i),
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::scene::SceneKind;
    use crate::OctansApp;
    use octans_core::{Shape, TOMBSTONE_TYPE};

    /// Two inverts wired in a chain, selected → captured. The unconnected required input and the
    /// unconsumed output become the template's 1-in/1-out boundary.
    fn app_with_invert_template() -> (OctansApp, usize, usize) {
        let mut app = OctansApp::from_scene(SceneKind::Diagnostics);
        let a = app.add_node_from_catalog("octans.image.invert").unwrap();
        let b = app.add_node_from_catalog("octans.image.invert").unwrap();
        assert!(app.try_connect(a, "image", b, "image"));
        // Feed the chain from the scene's camera so the whole graph compiles; the crossing edge
        // becomes the captured template's boundary input.
        let cam = octans_core::NodeId(0);
        assert!(app.try_connect(cam, "frame", a, "image"));
        app.sel_set.insert(a.0);
        app.sel_set.insert(b.0);
        app.capture_selection();
        assert_eq!(app.templates.len(), 1, "{:?}", app.edit_error);
        let spec = &app.templates[0].1;
        assert_eq!((spec.inputs.len(), spec.outputs.len()), (1, 1));
        (app, a.0, b.0)
    }

    #[test]
    fn capture_instantiate_and_undo() {
        let (mut app, _, _) = app_with_invert_template();
        let before = app.graph.node_count();
        app.instantiate_template(0);
        assert_eq!(app.graph.node_count(), before + 2, "two nodes inlined");
        let inlined = octans_core::NodeId(before);
        assert_eq!(
            app.graph.node(inlined).unwrap().node_type(),
            "octans.image.invert"
        );

        app.undo();
        assert_eq!(app.graph.node(inlined).unwrap().node_type(), TOMBSTONE_TYPE);
        app.redo();
        assert_eq!(
            app.graph.node(inlined).unwrap().node_type(),
            "octans.image.invert"
        );
    }

    #[test]
    fn loop_over_template_runs_and_map_vectorizes() {
        let (mut app, _, _) = app_with_invert_template();

        // Loop: 3 inversions of the camera frame == a single inversion.
        app.loop_count = 3;
        app.add_composite("loop", 0);
        assert!(app.edit_error.is_none(), "{:?}", app.edit_error);
        let lp = octans_core::NodeId(app.graph.node_count() - 1);
        assert_eq!(app.graph.node(lp).unwrap().node_type(), "octans.core.loop");
        let in_name = app.graph.node(lp).unwrap().inputs()[0].name;
        let out_name = app.graph.node(lp).unwrap().outputs()[0].name;
        let cam = octans_core::NodeId(0); // the diagnostics scene's camera
        assert!(app.try_connect(cam, "frame", lp, in_name));

        let engine = app.engine.as_mut().expect("compiles");
        let tick = engine.run_tick(&app.graph);
        let looped = tick
            .output(lp, out_name)
            .unwrap()
            .downcast_ref::<octans_nodes::Image>()
            .unwrap()
            .clone();
        let src = tick
            .output(cam, "frame")
            .unwrap()
            .downcast_ref::<octans_nodes::Image>()
            .unwrap()
            .clone();
        // The template body is two chained inverts, so each iteration is a double-inversion:
        // 3 iterations = 6 inversions = identity.
        assert_eq!(looped.px, src.px, "3 × (invert∘invert) = identity");

        // Map: the same template's boundary ports come out vectorized.
        app.add_composite("map", 0);
        let mp = octans_core::NodeId(app.graph.node_count() - 1);
        assert_eq!(app.graph.node(mp).unwrap().node_type(), "octans.core.map");
        assert!(matches!(
            app.graph.node(mp).unwrap().inputs()[0].ty.shape,
            Shape::Vector(_)
        ));

        // Loop undo tombstones it; redo rebuilds from the stored BodySpec.
        app.undo(); // remove map
        app.undo(); // undo the loop's input wire
        app.undo(); // remove loop
        assert_eq!(app.graph.node(lp).unwrap().node_type(), TOMBSTONE_TYPE);
        app.redo();
        assert_eq!(app.graph.node(lp).unwrap().node_type(), "octans.core.loop");
    }

    #[test]
    fn loop_guard_rejects_wrong_boundary_shape() {
        let mut app = OctansApp::from_scene(SceneKind::Diagnostics);
        let m = app.add_node_from_catalog("octans.math.math").unwrap();
        app.sel_set.insert(m.0);
        app.capture_selection();
        assert_eq!(app.templates.len(), 1);
        let n_before = app.graph.node_count();
        app.add_composite("loop", 0); // 0-in/1-out template → refused, not a panic
        assert!(app
            .edit_error
            .as_deref()
            .unwrap_or("")
            .contains("loop needs"));
        assert_eq!(app.graph.node_count(), n_before, "nothing was added");
    }
}
