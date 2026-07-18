use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;
use tokio::task::JoinSet;

use crate::db::Db;
use crate::mcp::{prefixed_tool_name, McpManager};
use crate::models::*;
use crate::ollama::{ChatMessage, OllamaClient, ToolDef};

const AGENT_PREAMBLE: &str = "You are one agent inside an automated multi-agent workflow. \
Your final message is passed verbatim as input to the next agent(s) in the workflow, \
so make it complete and self-contained. Do not ask questions back - there is no human in the loop.";

const JUDGE_PROMPT: &str = "You are a router inside a workflow engine. You are given the output \
of an agent and a condition. Decide whether the output satisfies the condition. \
Reply with exactly one word: YES or NO.";

/// One pending execution of a node: the payloads it should run with.
struct Activation {
    node_id: String,
    /// (section heading, payload)
    sections: Vec<(String, String)>,
}

pub struct Engine {
    pub db: Arc<Db>,
    pub ollama: Arc<OllamaClient>,
    pub mcp: Arc<McpManager>,
    pub events: broadcast::Sender<RunEvent>,
    pub default_model: String,
    pub default_temperature: f32,
    canceled: Mutex<HashSet<String>>,
}

impl Engine {
    pub fn new(
        db: Arc<Db>,
        ollama: Arc<OllamaClient>,
        mcp: Arc<McpManager>,
        events: broadcast::Sender<RunEvent>,
        default_model: String,
        default_temperature: f32,
    ) -> Self {
        Self {
            db,
            ollama,
            mcp,
            events,
            default_model,
            default_temperature,
            canceled: Mutex::new(HashSet::new()),
        }
    }

    fn emit(&self, run: &Run, node_id: Option<String>, kind: &str, data: Value) {
        let _ = self.events.send(RunEvent {
            run_id: run.id.clone(),
            workflow_id: run.workflow_id.clone(),
            node_id,
            kind: kind.to_string(),
            data,
            ts: now_rfc3339(),
        });
    }

    pub fn cancel(&self, run_id: &str) {
        self.canceled.lock().unwrap().insert(run_id.to_string());
    }

    fn is_canceled(&self, run_id: &str) -> bool {
        self.canceled.lock().unwrap().contains(run_id)
    }

    /// Validates the graph and kicks off an asynchronous run.
    pub fn start_run(
        self: &Arc<Self>,
        workflow_id: &str,
        trigger: &str,
        input: String,
    ) -> Result<Run> {
        let workflow: Workflow = self
            .db
            .get("workflows", workflow_id)?
            .ok_or_else(|| anyhow!("workflow {workflow_id} not found"))?;
        if workflow.graph.nodes.is_empty() {
            bail!("workflow '{}' has no nodes", workflow.name);
        }
        validate_graph(&workflow.graph)?;

        let run = Run {
            id: new_id(),
            workflow_id: workflow.id.clone(),
            workflow_name: workflow.name.clone(),
            status: "running".into(),
            trigger: trigger.into(),
            input: input.clone(),
            error: None,
            started_at: now_rfc3339(),
            finished_at: None,
        };
        self.db.put_run(&run)?;
        self.emit(&run, None, "run_started", json!({ "workflow_name": workflow.name }));

        let engine = self.clone();
        let run_clone = run.clone();
        tokio::spawn(async move {
            let result = engine.execute(&run_clone, &workflow, &input).await;
            let mut finished = run_clone.clone();
            finished.finished_at = Some(now_rfc3339());
            match result {
                Ok(()) => finished.status = "succeeded".into(),
                Err(e) => {
                    if engine.is_canceled(&run_clone.id) {
                        finished.status = "canceled".into();
                    } else {
                        finished.status = "failed".into();
                    }
                    finished.error = Some(format!("{e:#}"));
                }
            }
            if let Err(e) = engine.db.put_run(&finished) {
                tracing::error!("persisting run: {e:#}");
            }
            engine.canceled.lock().unwrap().remove(&run_clone.id);
            engine.emit(
                &finished,
                None,
                "run_finished",
                json!({ "status": finished.status, "error": finished.error }),
            );
        });
        Ok(run)
    }

    /// Activation-driven executor.
    ///
    /// Edges are classified into forward edges (a DAG) and loop-back edges.
    /// A node first runs once all its forward incoming edges have resolved
    /// and at least one of them fired; if none fired the node is skipped and
    /// the skip cascades. After its first activation, any firing edge
    /// (forward or back) re-activates the node with that payload alone.
    /// Every activation consumes one step of the workflow's loop budget.
    async fn execute(self: &Arc<Self>, run: &Run, workflow: &Workflow, input: &str) -> Result<()> {
        let graph = &workflow.graph;
        let nodes: HashMap<String, WorkflowNode> =
            graph.nodes.iter().map(|n| (n.id.clone(), n.clone())).collect();
        let back_edges = classify_back_edges(graph);
        let outgoing: HashMap<&str, Vec<&WorkflowEdge>> = {
            let mut m: HashMap<&str, Vec<&WorkflowEdge>> = HashMap::new();
            for e in &graph.edges {
                m.entry(e.source.as_str()).or_default().push(e);
            }
            m
        };
        // Forward incoming edge ids per node.
        let mut fwd_incoming: HashMap<&str, Vec<&WorkflowEdge>> = HashMap::new();
        for e in &graph.edges {
            if !back_edges.contains(&e.id) {
                fwd_incoming.entry(e.target.as_str()).or_default().push(e);
            }
        }
        // Nodes that may run more than once: loop heads (back-edge targets)
        // and everything reachable from them. A silent conditional edge out
        // of such a node is provisional - it may still fire on a later pass -
        // so it must not trigger an eager skip of its target.
        let can_rerun: HashSet<String> = {
            let mut set: HashSet<String> = HashSet::new();
            let mut stack: Vec<&str> = graph
                .edges
                .iter()
                .filter(|e| back_edges.contains(&e.id))
                .map(|e| e.target.as_str())
                .collect();
            while let Some(n) = stack.pop() {
                if set.insert(n.to_string()) {
                    for e in outgoing.get(n).cloned().unwrap_or_default() {
                        stack.push(e.target.as_str());
                    }
                }
            }
            set
        };

        let max_steps = if workflow.max_steps == 0 {
            DEFAULT_MAX_STEPS
        } else {
            workflow.max_steps
        };

        let agent_name = |node_id: &str| -> String {
            let Some(node) = nodes.get(node_id) else {
                return node_id.to_string();
            };
            if node.kind == "file" {
                return format!("File: {}", file_display_name(node));
            }
            self.db
                .get::<AgentCard>("agent_cards", &node.agent_card_id)
                .ok()
                .flatten()
                .map(|c| c.name)
                .unwrap_or_else(|| node_id.to_string())
        };

        // Heading of the input section a downstream agent sees for a payload
        // coming out of `node_id`.
        let section_label = |node_id: &str, via_loop: bool| -> String {
            match nodes.get(node_id) {
                Some(n) if n.kind == "file" => {
                    format!("Content of file \"{}\"", file_display_name(n))
                }
                _ => {
                    let name = agent_name(node_id);
                    if via_loop {
                        format!("Output from upstream agent \"{name}\" (loop iteration)")
                    } else {
                        format!("Output from upstream agent \"{name}\"")
                    }
                }
            }
        };

        // edge id -> Some(payload) if fired, None if resolved silent.
        let mut edge_state: HashMap<String, Option<String>> = HashMap::new();
        let mut first_activated: HashSet<String> = HashSet::new();
        let mut activations_count: HashMap<String, u32> = HashMap::new();
        let mut queue: VecDeque<Activation> = VecDeque::new();
        let mut busy: HashSet<String> = HashSet::new();
        let mut steps_used: u32 = 0;
        let mut tasks: JoinSet<(String, u32, Result<String>)> = JoinSet::new();
        let mut first_error: Option<anyhow::Error> = None;

        // Seed: nodes without forward incoming edges start with the workflow input.
        for node in &graph.nodes {
            if !fwd_incoming.contains_key(node.id.as_str()) {
                first_activated.insert(node.id.clone());
                let sections = if input.trim().is_empty() {
                    Vec::new()
                } else {
                    vec![("Workflow input".to_string(), input.to_string())]
                };
                queue.push_back(Activation {
                    node_id: node.id.clone(),
                    sections,
                });
            }
        }

        loop {
            if self.is_canceled(&run.id) && first_error.is_none() {
                first_error = Some(anyhow!("run canceled"));
            }

            // Start every queued activation whose node is not currently running.
            if first_error.is_none() {
                let mut deferred: VecDeque<Activation> = VecDeque::new();
                while let Some(act) = queue.pop_front() {
                    if busy.contains(&act.node_id) {
                        deferred.push_back(act);
                        continue;
                    }
                    steps_used += 1;
                    if steps_used > max_steps {
                        first_error = Some(anyhow!(
                            "loop budget exhausted: more than {max_steps} agent activations \
                             (raise the workflow's max steps if this is intended)"
                        ));
                        break;
                    }
                    let activation_no = {
                        let c = activations_count.entry(act.node_id.clone()).or_insert(0);
                        *c += 1;
                        *c
                    };
                    busy.insert(act.node_id.clone());
                    let node = nodes
                        .get(&act.node_id)
                        .ok_or_else(|| anyhow!("unknown node {}", act.node_id))?
                        .clone();
                    let node_input = render_input(&act.sections);
                    let engine = self.clone();
                    let run = run.clone();
                    tasks.spawn(async move {
                        let res = engine.run_node(&run, &node, node_input, activation_no).await;
                        (node.id.clone(), activation_no, res)
                    });
                }
                queue = deferred;
            }

            let Some(joined) = tasks.join_next().await else {
                break; // nothing running; queue is empty or we stopped on error
            };
            let (node_id, _activation_no, res) = joined.context("node task panicked")?;
            busy.remove(&node_id);

            match res {
                Ok(output) => {
                    // Resolve outgoing edges; skips cascade via this worklist.
                    let mut worklist: VecDeque<(String, Option<String>)> = VecDeque::new();
                    worklist.push_back((node_id, Some(output)));
                    while let Some((nid, output)) = worklist.pop_front() {
                        for edge in outgoing.get(nid.as_str()).cloned().unwrap_or_default() {
                            let taken = match &output {
                                None => false,
                                Some(out) => match self.evaluate_condition(edge, out).await {
                                    Ok(t) => t,
                                    Err(e) => {
                                        if first_error.is_none() {
                                            first_error = Some(e.context(format!(
                                                "evaluating condition on edge to \"{}\"",
                                                agent_name(&edge.target)
                                            )));
                                        }
                                        false
                                    }
                                },
                            };
                            self.emit(
                                run,
                                Some(nid.clone()),
                                "edge_resolved",
                                json!({ "edge_id": edge.id, "taken": taken }),
                            );
                            let is_back = back_edges.contains(&edge.id);
                            if is_back {
                                if taken && first_error.is_none() {
                                    queue.push_back(Activation {
                                        node_id: edge.target.clone(),
                                        sections: vec![(
                                            section_label(&nid, true),
                                            output.clone().unwrap_or_default(),
                                        )],
                                    });
                                }
                                continue;
                            }
                            // Forward edge.
                            if !taken && can_rerun.contains(&nid) {
                                // Provisional silence: the source sits in a loop
                                // and may fire this edge on a later pass.
                                continue;
                            }
                            edge_state.insert(
                                edge.id.clone(),
                                if taken { output.clone() } else { None },
                            );
                            let target = edge.target.clone();
                            if first_activated.contains(&target) {
                                if taken && first_error.is_none() {
                                    queue.push_back(Activation {
                                        node_id: target,
                                        sections: vec![(
                                            section_label(&nid, false),
                                            output.clone().unwrap_or_default(),
                                        )],
                                    });
                                }
                                continue;
                            }
                            let incoming = fwd_incoming.get(target.as_str()).cloned().unwrap_or_default();
                            if !incoming.iter().all(|e| edge_state.contains_key(&e.id)) {
                                continue; // still waiting for other inputs
                            }
                            first_activated.insert(target.clone());
                            let sections: Vec<(String, String)> = incoming
                                .iter()
                                .filter_map(|e| {
                                    edge_state.get(&e.id).and_then(|payload| {
                                        payload.as_ref().map(|p| {
                                            (section_label(&e.source, false), p.clone())
                                        })
                                    })
                                })
                                .collect();
                            if sections.is_empty() {
                                // No input fired: skip this node and cascade.
                                let nr = NodeRun {
                                    id: new_id(),
                                    run_id: run.id.clone(),
                                    node_id: target.clone(),
                                    agent_name: agent_name(&target),
                                    status: "skipped".into(),
                                    input: String::new(),
                                    output: String::new(),
                                    transcript: Value::Null,
                                    error: None,
                                    activation: 0,
                                    started_at: now_rfc3339(),
                                    finished_at: Some(now_rfc3339()),
                                };
                                let _ = self.db.put_node_run(&nr);
                                self.emit(
                                    run,
                                    Some(target.clone()),
                                    "node_skipped",
                                    json!({ "agent_name": nr.agent_name }),
                                );
                                worklist.push_back((target, None));
                            } else if first_error.is_none() {
                                queue.push_back(Activation { node_id: target, sections });
                            }
                        }
                    }
                }
                Err(e) => {
                    if first_error.is_none() {
                        first_error =
                            Some(e.context(format!("agent \"{}\" failed", agent_name(&node_id))));
                    }
                }
            }
        }

        if let Some(e) = first_error {
            return Err(e);
        }
        // Nodes that never got an input (e.g. a loop exited without firing
        // their edge) are marked skipped for visibility.
        for node in &graph.nodes {
            if first_activated.contains(&node.id) {
                continue;
            }
            let nr = NodeRun {
                id: new_id(),
                run_id: run.id.clone(),
                node_id: node.id.clone(),
                agent_name: agent_name(&node.id),
                status: "skipped".into(),
                input: String::new(),
                output: String::new(),
                transcript: Value::Null,
                error: None,
                activation: 0,
                started_at: now_rfc3339(),
                finished_at: Some(now_rfc3339()),
            };
            let _ = self.db.put_node_run(&nr);
            self.emit(
                run,
                Some(node.id.clone()),
                "node_skipped",
                json!({ "agent_name": nr.agent_name }),
            );
        }
        Ok(())
    }

    /// Decides whether an edge fires for the given source output.
    async fn evaluate_condition(&self, edge: &WorkflowEdge, output: &str) -> Result<bool> {
        let base = match edge.condition_kind.as_str() {
            "" | "always" => true,
            "contains" => output
                .to_lowercase()
                .contains(&edge.condition.to_lowercase()),
            "regex" => regex::Regex::new(&edge.condition)
                .with_context(|| format!("invalid regex '{}'", edge.condition))?
                .is_match(output),
            "llm" => {
                let messages = vec![
                    ChatMessage::new("system", JUDGE_PROMPT),
                    ChatMessage::new(
                        "user",
                        format!(
                            "Condition: {}\n\nAgent output:\n{}\n\nDoes the output satisfy the condition? YES or NO.",
                            edge.condition, output
                        ),
                    ),
                ];
                let reply = self
                    .ollama
                    .chat(&self.default_model, &messages, &[], 0.0)
                    .await
                    .context("LLM condition check failed")?;
                let upper = reply.content.to_uppercase();
                match (upper.find("YES"), upper.find("NO")) {
                    (Some(y), Some(n)) => y < n,
                    (Some(_), None) => true,
                    (None, Some(_)) => false,
                    (None, None) => {
                        tracing::warn!(
                            "condition judge gave no YES/NO (\"{}\") - treating as NO",
                            reply.content
                        );
                        false
                    }
                }
            }
            other => bail!("unknown edge condition kind '{other}'"),
        };
        Ok(base != edge.negate)
    }

    /// Executes one activation of a node. Agent nodes run an LLM loop with
    /// MCP tool dispatch; file nodes read their file and emit its content.
    async fn run_node(
        self: &Arc<Self>,
        run: &Run,
        node: &WorkflowNode,
        input: String,
        activation: u32,
    ) -> Result<String> {
        let card: Option<AgentCard> = if node.kind == "file" {
            None
        } else {
            Some(
                self.db
                    .get("agent_cards", &node.agent_card_id)?
                    .ok_or_else(|| anyhow!("agent card {} not found", node.agent_card_id))?,
            )
        };
        let (display_name, input_shown) = match &card {
            None => (format!("File: {}", file_display_name(node)), node.file_path.clone()),
            Some(card) => (card.name.clone(), input.clone()),
        };

        let mut node_run = NodeRun {
            id: new_id(),
            run_id: run.id.clone(),
            node_id: node.id.clone(),
            agent_name: display_name.clone(),
            status: "running".into(),
            input: input_shown,
            output: String::new(),
            transcript: Value::Null,
            error: None,
            activation,
            started_at: now_rfc3339(),
            finished_at: None,
        };
        self.db.put_node_run(&node_run)?;
        self.emit(
            run,
            Some(node.id.clone()),
            "node_started",
            json!({ "agent_name": display_name, "activation": activation }),
        );

        let result = match &card {
            None => tokio::fs::read_to_string(&node.file_path)
                .await
                .with_context(|| format!("reading file '{}'", node.file_path))
                .map(|content| (content, Value::Null)),
            Some(card) => self.agent_loop(run, node, card, &input).await,
        };
        node_run.finished_at = Some(now_rfc3339());
        let output = match result {
            Ok((output, transcript)) => {
                node_run.status = "succeeded".into();
                node_run.output = output.clone();
                node_run.transcript = transcript;
                Ok(output)
            }
            Err(e) => {
                node_run.status = "failed".into();
                node_run.error = Some(format!("{e:#}"));
                Err(e)
            }
        };
        self.db.put_node_run(&node_run)?;
        self.emit(
            run,
            Some(node.id.clone()),
            "node_finished",
            json!({
                "status": node_run.status,
                "error": node_run.error,
                "output": node_run.output,
                "activation": activation,
            }),
        );
        output
    }

    async fn agent_loop(
        self: &Arc<Self>,
        run: &Run,
        node: &WorkflowNode,
        card: &AgentCard,
        input: &str,
    ) -> Result<(String, Value)> {
        // Collect tools from every MCP server on the card.
        let mut tools: Vec<ToolDef> = Vec::new();
        let mut dispatch: HashMap<String, (McpServerConfig, String)> = HashMap::new();
        for server_id in &card.mcp_servers {
            let Some(cfg): Option<McpServerConfig> = self.db.get("mcp_servers", server_id)? else {
                tracing::warn!("agent '{}': MCP server {server_id} no longer exists", card.name);
                continue;
            };
            if !cfg.enabled {
                continue;
            }
            let server_tools = self
                .mcp
                .list_tools(&cfg)
                .await
                .with_context(|| format!("listing tools of MCP server '{}'", cfg.name))?;
            for tool in server_tools {
                let exposed = prefixed_tool_name(&cfg.name, &tool.name);
                dispatch.insert(exposed.clone(), (cfg.clone(), tool.name.clone()));
                tools.push(ToolDef::new(exposed, tool.description, tool.input_schema));
            }
        }

        let mut system = String::from(AGENT_PREAMBLE);
        if !card.system_prompt.trim().is_empty() {
            system.push_str("\n\n");
            system.push_str(&card.system_prompt);
        }
        if !node.instructions.trim().is_empty() {
            system.push_str("\n\n## Your task in this workflow\n");
            system.push_str(&node.instructions);
        }

        let model = card
            .model
            .clone()
            .filter(|m| !m.trim().is_empty())
            .unwrap_or_else(|| self.default_model.clone());
        let temperature = card.temperature.unwrap_or(self.default_temperature);
        let max_iter = card.max_tool_iterations.max(1);

        let mut messages = vec![
            ChatMessage::new("system", system),
            ChatMessage::new("user", input),
        ];

        for _ in 0..max_iter {
            if self.is_canceled(&run.id) {
                bail!("run canceled");
            }
            let reply = self
                .ollama
                .chat(&model, &messages, &tools, temperature)
                .await?;
            messages.push(reply.clone());
            let calls = reply.tool_calls.clone().unwrap_or_default();
            if calls.is_empty() {
                return Ok((reply.content, serde_json::to_value(&messages)?));
            }
            self.emit(
                run,
                Some(node.id.clone()),
                "agent_step",
                json!({ "tool_calls": calls.len(), "content": reply.content }),
            );
            for call in calls {
                let name = call.function.name.clone();
                let mut args = call.function.arguments.clone();
                if let Some(s) = args.as_str() {
                    args = serde_json::from_str(s).unwrap_or(json!({}));
                }
                self.emit(
                    run,
                    Some(node.id.clone()),
                    "tool_call",
                    json!({ "tool": name, "arguments": args }),
                );
                let result_text = match dispatch.get(&name) {
                    Some((cfg, original)) => {
                        match self.mcp.call_tool(cfg, original, args).await {
                            Ok(t) if t.trim().is_empty() => "(tool returned no output)".to_string(),
                            Ok(t) => t,
                            Err(e) => format!("ERROR calling tool: {e:#}"),
                        }
                    }
                    None => format!("ERROR: unknown tool '{name}'"),
                };
                let mut tool_msg = ChatMessage::new("tool", result_text);
                tool_msg.tool_name = Some(name);
                messages.push(tool_msg);
            }
        }

        // Tool budget exhausted: force a final answer without tools.
        messages.push(ChatMessage::new(
            "user",
            "Tool budget exhausted. Produce your final result now based on what you have.",
        ));
        let reply = self.ollama.chat(&model, &messages, &[], temperature).await?;
        messages.push(reply.clone());
        Ok((reply.content, serde_json::to_value(&messages)?))
    }
}

fn render_input(sections: &[(String, String)]) -> String {
    if sections.is_empty() {
        return "Execute your task.".to_string();
    }
    sections
        .iter()
        .map(|(heading, payload)| format!("# {heading}\n{payload}"))
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Short display name of a file node: the file name portion of its path.
fn file_display_name(node: &WorkflowNode) -> String {
    std::path::Path::new(&node.file_path)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| node.file_path.clone())
}

/// DFS edge classification: an edge pointing at a node on the current DFS
/// stack closes a cycle and becomes a loop-back edge. Removing all back
/// edges leaves an acyclic forward graph. Roots are the nodes without
/// incoming edges (in declaration order), then any still-unvisited node.
fn classify_back_edges(graph: &Graph) -> HashSet<String> {
    const WHITE: u8 = 0;
    const GRAY: u8 = 1;
    const BLACK: u8 = 2;

    let mut adj: HashMap<&str, Vec<&WorkflowEdge>> = HashMap::new();
    let mut has_incoming: HashSet<&str> = HashSet::new();
    for e in &graph.edges {
        adj.entry(e.source.as_str()).or_default().push(e);
        has_incoming.insert(e.target.as_str());
    }

    let mut color: HashMap<&str, u8> = graph.nodes.iter().map(|n| (n.id.as_str(), WHITE)).collect();
    let mut back: HashSet<String> = HashSet::new();

    let roots: Vec<&str> = graph
        .nodes
        .iter()
        .map(|n| n.id.as_str())
        .filter(|id| !has_incoming.contains(id))
        .chain(graph.nodes.iter().map(|n| n.id.as_str()))
        .collect();

    for root in roots {
        if color.get(root).copied().unwrap_or(BLACK) != WHITE {
            continue;
        }
        let mut stack: Vec<(&str, usize)> = vec![(root, 0)];
        color.insert(root, GRAY);
        while let Some((node, idx)) = stack.pop() {
            let edges = adj.get(node).cloned().unwrap_or_default();
            if idx < edges.len() {
                stack.push((node, idx + 1));
                let e = edges[idx];
                match color.get(e.target.as_str()).copied().unwrap_or(BLACK) {
                    WHITE => {
                        color.insert(e.target.as_str(), GRAY);
                        stack.push((e.target.as_str(), 0));
                    }
                    GRAY => {
                        back.insert(e.id.clone());
                    }
                    _ => {}
                }
            } else {
                color.insert(node, BLACK);
            }
        }
    }
    back
}

/// Structural validation. Cycles are allowed (they become loops at runtime,
/// bounded by the workflow's step budget); edges must reference existing
/// nodes and conditions must be well-formed.
pub fn validate_graph(graph: &Graph) -> Result<()> {
    for n in &graph.nodes {
        match n.kind.as_str() {
            "" | "agent" => {
                if n.agent_card_id.trim().is_empty() {
                    bail!("node {}: no agent card selected", n.id);
                }
            }
            "file" => {
                if n.file_path.trim().is_empty() {
                    bail!("file node {}: no file path set", n.id);
                }
            }
            other => bail!("node {}: unknown node kind '{other}'", n.id),
        }
    }
    let ids: HashSet<&str> = graph.nodes.iter().map(|n| n.id.as_str()).collect();
    for e in &graph.edges {
        if !ids.contains(e.source.as_str()) || !ids.contains(e.target.as_str()) {
            bail!("edge {} -> {} references a missing node", e.source, e.target);
        }
        match e.condition_kind.as_str() {
            "" | "always" => {}
            "contains" | "llm" => {
                if e.condition.trim().is_empty() {
                    bail!(
                        "edge {} -> {}: condition kind '{}' needs a condition text",
                        e.source,
                        e.target,
                        e.condition_kind
                    );
                }
            }
            "regex" => {
                regex::Regex::new(&e.condition).with_context(|| {
                    format!("edge {} -> {}: invalid regex '{}'", e.source, e.target, e.condition)
                })?;
            }
            other => bail!("edge {} -> {}: unknown condition kind '{other}'", e.source, e.target),
        }
    }
    Ok(())
}
