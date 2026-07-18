use serde::{Deserialize, Serialize};
use std::collections::HashMap;

fn default_max_iter() -> u32 {
    10
}

/// An agent card fully describes one agent: its role (system prompt), the
/// model it runs on and the MCP servers whose tools it may use.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentCard {
    #[serde(default)]
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// Optional model override; falls back to the server-wide default.
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub system_prompt: String,
    /// Ids of MCP servers whose tools this agent may call.
    #[serde(default)]
    pub mcp_servers: Vec<String>,
    #[serde(default)]
    pub temperature: Option<f32>,
    #[serde(default = "default_max_iter")]
    pub max_tool_iterations: u32,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
pub struct Position {
    pub x: f64,
    pub y: f64,
}

fn default_node_kind() -> String {
    "agent".into()
}

/// A node in a workflow graph. Kind "agent" is an instance of an agent card
/// plus workflow-specific instructions; kind "file" reads a local text file
/// and emits its content as output; kind "file_dest" writes its input to a
/// local file and passes the content through unchanged.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowNode {
    pub id: String,
    /// "agent" | "file" | "file_dest"
    #[serde(default = "default_node_kind")]
    pub kind: String,
    #[serde(default)]
    pub agent_card_id: String,
    /// Task description for this agent within this workflow. Appended to the
    /// agent card's system prompt.
    #[serde(default)]
    pub instructions: String,
    /// kinds "file"/"file_dest": absolute or server-relative path of the
    /// text file to read from / write to.
    #[serde(default)]
    pub file_path: String,
    /// kind "file_dest": append to the file instead of overwriting it.
    #[serde(default)]
    pub append: bool,
    #[serde(default)]
    pub position: Position,
}

fn default_condition_kind() -> String {
    "always".into()
}

/// A directed edge: output of `source` becomes input of `target`.
/// If a condition is set, the edge only fires when the source output
/// satisfies it; a node whose incoming edges all stay silent is skipped.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowEdge {
    pub id: String,
    pub source: String,
    pub target: String,
    /// "always" | "contains" | "regex" | "llm"
    #[serde(default = "default_condition_kind")]
    pub condition_kind: String,
    /// Substring, regex pattern, or natural-language predicate.
    #[serde(default)]
    pub condition: String,
    /// Invert the condition result (for else-branches).
    #[serde(default)]
    pub negate: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Graph {
    #[serde(default)]
    pub nodes: Vec<WorkflowNode>,
    #[serde(default)]
    pub edges: Vec<WorkflowEdge>,
}

pub const DEFAULT_MAX_STEPS: u32 = 25;

fn default_max_steps() -> u32 {
    DEFAULT_MAX_STEPS
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workflow {
    #[serde(default)]
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub graph: Graph,
    /// Loop budget: maximum number of agent activations in one run.
    #[serde(default = "default_max_steps")]
    pub max_steps: u32,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
}

fn default_transport() -> String {
    "stdio".into()
}

/// Connection settings for one MCP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    #[serde(default)]
    pub id: String,
    pub name: String,
    /// "stdio" or "http"
    #[serde(default = "default_transport")]
    pub transport: String,
    /// stdio: executable to spawn
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// http: endpoint URL (streamable HTTP transport)
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Schedule {
    #[serde(default)]
    pub id: String,
    pub workflow_id: String,
    /// 6-field cron expression (sec min hour day month weekday);
    /// 5-field expressions are normalized by prepending "0 ".
    pub cron: String,
    /// Input text handed to the workflow's source nodes on each run.
    #[serde(default)]
    pub input: String,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub last_run_at: Option<String>,
    #[serde(default)]
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Run {
    pub id: String,
    pub workflow_id: String,
    #[serde(default)]
    pub workflow_name: String,
    /// pending | running | succeeded | failed | canceled
    pub status: String,
    /// manual | schedule
    #[serde(default)]
    pub trigger: String,
    #[serde(default)]
    pub input: String,
    #[serde(default)]
    pub error: Option<String>,
    pub started_at: String,
    #[serde(default)]
    pub finished_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeRun {
    pub id: String,
    pub run_id: String,
    pub node_id: String,
    #[serde(default)]
    pub agent_name: String,
    /// pending | running | succeeded | failed | skipped
    pub status: String,
    /// 1-based activation counter: >1 means the node re-ran in a loop.
    #[serde(default)]
    pub activation: u32,
    #[serde(default)]
    pub input: String,
    #[serde(default)]
    pub output: String,
    /// Full message transcript (system/user/assistant/tool) as JSON.
    #[serde(default)]
    pub transcript: serde_json::Value,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub started_at: String,
    #[serde(default)]
    pub finished_at: Option<String>,
}

/// Live event broadcast over SSE while runs execute.
#[derive(Debug, Clone, Serialize)]
pub struct RunEvent {
    pub run_id: String,
    pub workflow_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
    /// run_started | run_finished | node_started | node_finished | tool_call | agent_step
    pub kind: String,
    pub data: serde_json::Value,
    pub ts: String,
}

pub fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

pub fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}
