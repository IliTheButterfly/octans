//! Graph (de)serialization — the **IR as data**.
//!
//! A [`Graph`] is a live object graph; its serializable form is a [`GraphSpec`]: each node as a
//! `(type-id, config-json)` pair plus the edges. To rebuild a graph you supply a [`NodeRegistry`]
//! that maps a node-type-id to a factory `config -> Box<dyn Node>`. The same spec is what a
//! future codegen backend lowers.
//!
//! v1 covers graphs of plain registered nodes. Closure-carrying constructs (`Map`/`Strategy`/
//! `group` bodies) and `Portal`s aren't captured yet — their `to_json` is `null`.

use crate::graph::{ConnectError, Graph};
use crate::node::Node;
use crate::registry::Registry;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct NodeSpec {
    #[serde(rename = "type")]
    pub type_id: String,
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub config: serde_json::Value,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct EdgeSpec {
    pub from: usize,
    pub from_port: String,
    pub to: usize,
    pub to_port: String,
}

/// The serializable form of a graph.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct GraphSpec {
    pub nodes: Vec<NodeSpec>,
    pub edges: Vec<EdgeSpec>,
}

type Factory = Box<dyn Fn(&serde_json::Value) -> Box<dyn Node>>;

/// Maps a node-type-id to a factory that builds the node from its config JSON.
#[derive(Default)]
pub struct NodeRegistry {
    factories: HashMap<String, Factory>,
}

impl NodeRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(
        &mut self,
        type_id: &str,
        factory: impl Fn(&serde_json::Value) -> Box<dyn Node> + 'static,
    ) {
        self.factories
            .insert(type_id.to_string(), Box::new(factory));
    }

    /// Register a `serde`-deserializable node by type id (config is `serde_json::from_value`'d).
    pub fn register_serde<T>(&mut self, type_id: &str)
    where
        T: Node + serde::de::DeserializeOwned + 'static,
    {
        self.register(type_id, |cfg| {
            Box::new(serde_json::from_value::<T>(cfg.clone()).expect("node config deserializes"))
        });
    }
}

#[derive(Debug)]
pub enum BuildError {
    UnknownNodeType(String),
    Connect(ConnectError),
}

impl Graph {
    /// Capture this graph as a [`GraphSpec`] (nodes + edges; see module note on what's covered).
    pub fn to_spec(&self) -> GraphSpec {
        let nodes = self
            .nodes
            .iter()
            .map(|n| NodeSpec {
                type_id: n.node_type().to_string(),
                config: n.to_json(),
            })
            .collect();
        let edges = self
            .edges
            .iter()
            .map(|e| EdgeSpec {
                from: e.from_node,
                from_port: e.from_port.to_string(),
                to: e.to_node,
                to_port: e.to_port.to_string(),
            })
            .collect();
        GraphSpec { nodes, edges }
    }
}

impl GraphSpec {
    /// Rebuild a runnable [`Graph`]: construct each node via `factories`, then re-make every edge
    /// (which re-runs connect-time type checking against `registry`).
    pub fn build(&self, registry: Registry, factories: &NodeRegistry) -> Result<Graph, BuildError> {
        let mut g = Graph::new(registry);
        for ns in &self.nodes {
            let factory = factories
                .factories
                .get(&ns.type_id)
                .ok_or_else(|| BuildError::UnknownNodeType(ns.type_id.clone()))?;
            g.add_boxed(factory(&ns.config));
        }
        for es in &self.edges {
            g.connect(
                crate::graph::NodeId(es.from),
                &es.from_port,
                crate::graph::NodeId(es.to),
                &es.to_port,
            )
            .map_err(BuildError::Connect)?;
        }
        Ok(g)
    }
}
