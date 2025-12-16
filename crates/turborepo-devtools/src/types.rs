//! Serializable types for the devtools WebSocket protocol.
//!
//! These types define the messages exchanged between the CLI server
//! and the web client, as well as the graph data structures.

use serde::{Deserialize, Serialize};

/// Messages sent from CLI server to web client
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ServerMessage {
    /// Initial state on connection
    Init { data: GraphState },
    /// Updated state after file changes
    Update { data: GraphState },
    /// Keep-alive ping
    Ping,
    /// Error message
    Error { message: String },
}

/// Messages sent from web client to CLI server
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ClientMessage {
    /// Pong response to ping
    Pong,
}

/// Full graph state sent to clients
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphState {
    /// The package dependency graph
    pub package_graph: PackageGraphData,
    /// The task dependency graph (tasks and their dependencies based on
    /// turbo.json)
    pub task_graph: TaskGraphData,
    /// Absolute path to the repository root
    pub repo_root: String,
    /// Version of turbo running the devtools
    pub turbo_version: String,
}

/// Package dependency graph in a serializable format
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageGraphData {
    /// All packages in the monorepo
    pub nodes: Vec<PackageNode>,
    /// Dependency edges between packages
    pub edges: Vec<GraphEdge>,
}

/// A package in the dependency graph
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageNode {
    /// Unique identifier (package name, or "__ROOT__" for root)
    pub id: String,
    /// Display name
    pub name: String,
    /// Path relative to repo root
    pub path: String,
    /// Available npm scripts
    pub scripts: Vec<String>,
    /// Is this the root package?
    pub is_root: bool,
}

/// An edge in the graph representing a dependency relationship
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphEdge {
    /// Source node ID (the dependent package)
    pub source: String,
    /// Target node ID (the dependency)
    pub target: String,
}

/// Task dependency graph in a serializable format
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskGraphData {
    /// All task nodes in the graph
    pub nodes: Vec<TaskNode>,
    /// Dependency edges between tasks
    pub edges: Vec<GraphEdge>,
}

/// A task node in the task graph
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskNode {
    /// Unique identifier (package#task format)
    pub id: String,
    /// Package name this task belongs to
    pub package: String,
    /// Task name (e.g., "build", "test")
    pub task: String,
}
