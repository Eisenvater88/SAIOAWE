use anyhow::{anyhow, bail, Context, Result};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{oneshot, Mutex};

use crate::models::McpServerConfig;

pub const PROTOCOL_VERSION: &str = "2024-11-05";

#[derive(Debug, Clone, Serialize)]
pub struct McpTool {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

/// Manages live connections to configured MCP servers. Connections are
/// created lazily and kept alive; a changed config causes a reconnect.
pub struct McpManager {
    conns: Mutex<HashMap<String, Arc<McpConnection>>>,
    tool_timeout: Duration,
}

impl McpManager {
    pub fn new(tool_timeout: Duration) -> Self {
        Self {
            conns: Mutex::new(HashMap::new()),
            tool_timeout,
        }
    }

    async fn connection(&self, cfg: &McpServerConfig) -> Result<Arc<McpConnection>> {
        let mut conns = self.conns.lock().await;
        if let Some(existing) = conns.get(&cfg.id) {
            if existing.cfg.updated_at == cfg.updated_at && existing.is_alive().await {
                return Ok(existing.clone());
            }
            conns.remove(&cfg.id);
        }
        let conn = Arc::new(
            McpConnection::connect(cfg.clone(), self.tool_timeout)
                .await
                .with_context(|| format!("connecting to MCP server '{}'", cfg.name))?,
        );
        conns.insert(cfg.id.clone(), conn.clone());
        Ok(conn)
    }

    pub async fn disconnect(&self, server_id: &str) {
        self.conns.lock().await.remove(server_id);
    }

    pub async fn list_tools(&self, cfg: &McpServerConfig) -> Result<Vec<McpTool>> {
        let conn = self.connection(cfg).await?;
        conn.list_tools().await
    }

    pub async fn call_tool(&self, cfg: &McpServerConfig, name: &str, args: Value) -> Result<String> {
        let conn = self.connection(cfg).await?;
        conn.call_tool(name, args).await
    }
}

pub struct McpConnection {
    cfg: McpServerConfig,
    transport: Transport,
    next_id: AtomicI64,
    timeout: Duration,
    tools_cache: Mutex<Option<Vec<McpTool>>>,
}

enum Transport {
    Stdio(StdioTransport),
    Http(HttpTransport),
}

impl McpConnection {
    async fn connect(cfg: McpServerConfig, timeout: Duration) -> Result<Self> {
        let transport = match cfg.transport.as_str() {
            "stdio" => Transport::Stdio(StdioTransport::spawn(&cfg).await?),
            "http" => Transport::Http(HttpTransport::new(&cfg, timeout)?),
            other => bail!("unknown MCP transport '{other}' (use 'stdio' or 'http')"),
        };
        let conn = Self {
            cfg,
            transport,
            next_id: AtomicI64::new(1),
            timeout,
            tools_cache: Mutex::new(None),
        };
        conn.initialize().await?;
        Ok(conn)
    }

    async fn is_alive(&self) -> bool {
        match &self.transport {
            Transport::Stdio(t) => t.is_alive().await,
            Transport::Http(_) => true,
        }
    }

    async fn initialize(&self) -> Result<()> {
        let result = self
            .request(
                "initialize",
                json!({
                    "protocolVersion": PROTOCOL_VERSION,
                    "capabilities": {},
                    "clientInfo": { "name": "saioawe", "version": env!("CARGO_PKG_VERSION") }
                }),
            )
            .await
            .context("MCP initialize handshake failed")?;
        if let Transport::Http(t) = &self.transport {
            if let Some(v) = result.get("protocolVersion").and_then(|v| v.as_str()) {
                *t.protocol_version.lock().unwrap() = Some(v.to_string());
            }
        }
        self.notify("notifications/initialized", json!({})).await?;
        Ok(())
    }

    pub async fn list_tools(&self) -> Result<Vec<McpTool>> {
        {
            let cache = self.tools_cache.lock().await;
            if let Some(tools) = cache.as_ref() {
                return Ok(tools.clone());
            }
        }
        let result = self.request("tools/list", json!({})).await?;
        let raw = result
            .get("tools")
            .and_then(|t| t.as_array())
            .cloned()
            .unwrap_or_default();
        let tools: Vec<McpTool> = raw
            .iter()
            .filter_map(|t| {
                Some(McpTool {
                    name: t.get("name")?.as_str()?.to_string(),
                    description: t
                        .get("description")
                        .and_then(|d| d.as_str())
                        .unwrap_or("")
                        .to_string(),
                    input_schema: t
                        .get("inputSchema")
                        .cloned()
                        .unwrap_or_else(|| json!({"type": "object", "properties": {}})),
                })
            })
            .collect();
        *self.tools_cache.lock().await = Some(tools.clone());
        Ok(tools)
    }

    /// Calls a tool and flattens the result content into plain text.
    pub async fn call_tool(&self, name: &str, args: Value) -> Result<String> {
        let args = if args.is_object() { args } else { json!({}) };
        let result = self
            .request("tools/call", json!({ "name": name, "arguments": args }))
            .await?;
        let mut parts: Vec<String> = Vec::new();
        if let Some(content) = result.get("content").and_then(|c| c.as_array()) {
            for item in content {
                match item.get("type").and_then(|t| t.as_str()) {
                    Some("text") => {
                        if let Some(text) = item.get("text").and_then(|t| t.as_str()) {
                            parts.push(text.to_string());
                        }
                    }
                    Some(other) => parts.push(format!("[unsupported content of type '{other}']")),
                    None => {}
                }
            }
        }
        if parts.is_empty() {
            if let Some(sc) = result.get("structuredContent") {
                parts.push(serde_json::to_string_pretty(sc)?);
            }
        }
        let text = parts.join("\n");
        if result.get("isError").and_then(|e| e.as_bool()).unwrap_or(false) {
            bail!("tool '{name}' reported an error: {text}");
        }
        Ok(text)
    }

    async fn request(&self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let msg = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        let response = match &self.transport {
            Transport::Stdio(t) => t.request(id, &msg, self.timeout).await?,
            Transport::Http(t) => t.request(id, &msg).await?,
        };
        if let Some(err) = response.get("error") {
            bail!("MCP server '{}' error on {method}: {err}", self.cfg.name);
        }
        Ok(response.get("result").cloned().unwrap_or(Value::Null))
    }

    async fn notify(&self, method: &str, params: Value) -> Result<()> {
        let msg = json!({ "jsonrpc": "2.0", "method": method, "params": params });
        match &self.transport {
            Transport::Stdio(t) => t.send(&msg).await,
            Transport::Http(t) => t.notify(&msg).await,
        }
    }
}

// ---------------------------------------------------------------- stdio

struct StdioTransport {
    stdin: Mutex<ChildStdin>,
    child: Mutex<Child>,
    pending: Arc<StdMutex<HashMap<i64, oneshot::Sender<Value>>>>,
}

impl StdioTransport {
    async fn spawn(cfg: &McpServerConfig) -> Result<Self> {
        if cfg.command.trim().is_empty() {
            bail!("stdio MCP server '{}' has no command configured", cfg.name);
        }
        let mut child = Self::spawn_child(cfg)?;
        let stdin = child.stdin.take().context("no stdin on MCP child")?;
        let stdout = child.stdout.take().context("no stdout on MCP child")?;
        let stderr = child.stderr.take();

        let pending: Arc<StdMutex<HashMap<i64, oneshot::Sender<Value>>>> =
            Arc::new(StdMutex::new(HashMap::new()));

        // Route JSON-RPC responses from stdout to their waiting requests.
        let pending_reader = pending.clone();
        let server_name = cfg.name.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let line = line.trim().to_string();
                if line.is_empty() {
                    continue;
                }
                let Ok(value) = serde_json::from_str::<Value>(&line) else {
                    tracing::warn!("mcp[{server_name}] non-JSON line on stdout: {line}");
                    continue;
                };
                if let Some(id) = value.get("id").and_then(|i| i.as_i64()) {
                    if value.get("result").is_some() || value.get("error").is_some() {
                        if let Some(tx) = pending_reader.lock().unwrap().remove(&id) {
                            let _ = tx.send(value);
                        }
                        continue;
                    }
                }
                // Server-initiated requests/notifications are ignored in v1.
                tracing::debug!("mcp[{server_name}] unhandled message: {value}");
            }
            tracing::info!("mcp[{server_name}] stdout closed");
        });

        if let Some(stderr) = stderr {
            let server_name = cfg.name.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    tracing::info!("mcp[{server_name}] stderr: {line}");
                }
            });
        }

        Ok(Self {
            stdin: Mutex::new(stdin),
            child: Mutex::new(child),
            pending,
        })
    }

    fn spawn_child(cfg: &McpServerConfig) -> Result<Child> {
        let build = |program: &str, prefix_args: &[&str]| {
            let mut cmd = Command::new(program);
            cmd.args(prefix_args)
                .arg(&cfg.command)
                .args(&cfg.args)
                .envs(&cfg.env)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true);
            cmd
        };
        let mut direct = Command::new(&cfg.command);
        direct
            .args(&cfg.args)
            .envs(&cfg.env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        match direct.spawn() {
            Ok(child) => Ok(child),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && cfg!(windows) => {
                // Batch shims like npx.cmd / uvx.cmd need the shell on Windows.
                build("cmd", &["/C"])
                    .spawn()
                    .with_context(|| format!("spawning '{}' (also tried via cmd /C)", cfg.command))
            }
            Err(e) => Err(anyhow!("spawning '{}': {e}", cfg.command)),
        }
    }

    async fn is_alive(&self) -> bool {
        match self.child.lock().await.try_wait() {
            Ok(None) => true,
            _ => false,
        }
    }

    async fn send(&self, msg: &Value) -> Result<()> {
        let mut line = serde_json::to_string(msg)?;
        line.push('\n');
        let mut stdin = self.stdin.lock().await;
        stdin.write_all(line.as_bytes()).await?;
        stdin.flush().await?;
        Ok(())
    }

    async fn request(&self, id: i64, msg: &Value, timeout: Duration) -> Result<Value> {
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id, tx);
        if let Err(e) = self.send(msg).await {
            self.pending.lock().unwrap().remove(&id);
            return Err(e);
        }
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(_)) => bail!("MCP server closed the connection"),
            Err(_) => {
                self.pending.lock().unwrap().remove(&id);
                bail!("MCP request timed out after {}s", timeout.as_secs());
            }
        }
    }
}

// ---------------------------------------------------------------- http

struct HttpTransport {
    client: reqwest::Client,
    url: String,
    headers: HashMap<String, String>,
    session_id: StdMutex<Option<String>>,
    protocol_version: StdMutex<Option<String>>,
}

impl HttpTransport {
    fn new(cfg: &McpServerConfig, timeout: Duration) -> Result<Self> {
        if cfg.url.trim().is_empty() {
            bail!("http MCP server '{}' has no URL configured", cfg.name);
        }
        Ok(Self {
            client: reqwest::Client::builder().timeout(timeout).build()?,
            url: cfg.url.clone(),
            headers: cfg.headers.clone(),
            session_id: StdMutex::new(None),
            protocol_version: StdMutex::new(None),
        })
    }

    fn build_post(&self, msg: &Value) -> reqwest::RequestBuilder {
        let mut req = self
            .client
            .post(&self.url)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream")
            .json(msg);
        if let Some(sid) = self.session_id.lock().unwrap().clone() {
            req = req.header("Mcp-Session-Id", sid);
        }
        if let Some(pv) = self.protocol_version.lock().unwrap().clone() {
            req = req.header("MCP-Protocol-Version", pv);
        }
        for (k, v) in &self.headers {
            req = req.header(k, v);
        }
        req
    }

    fn capture_session(&self, resp: &reqwest::Response) {
        if let Some(sid) = resp
            .headers()
            .get("mcp-session-id")
            .and_then(|v| v.to_str().ok())
        {
            *self.session_id.lock().unwrap() = Some(sid.to_string());
        }
    }

    async fn notify(&self, msg: &Value) -> Result<()> {
        let resp = self.build_post(msg).send().await?;
        self.capture_session(&resp);
        Ok(())
    }

    async fn request(&self, id: i64, msg: &Value) -> Result<Value> {
        let resp = self.build_post(msg).send().await?;
        self.capture_session(&resp);
        let status = resp.status();
        let content_type = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        let body = resp.text().await?;
        if !status.is_success() {
            bail!("MCP HTTP endpoint returned {status}: {body}");
        }
        if content_type.starts_with("text/event-stream") {
            // Scan SSE events for the JSON-RPC response with our id.
            let mut data = String::new();
            for line in body.lines().chain(std::iter::once("")) {
                if let Some(rest) = line.strip_prefix("data:") {
                    data.push_str(rest.trim_start());
                    data.push('\n');
                } else if line.is_empty() && !data.is_empty() {
                    if let Ok(value) = serde_json::from_str::<Value>(data.trim()) {
                        if value.get("id").and_then(|i| i.as_i64()) == Some(id) {
                            return Ok(value);
                        }
                    }
                    data.clear();
                }
            }
            bail!("no JSON-RPC response for request {id} in SSE stream");
        }
        serde_json::from_str(&body).context("invalid JSON from MCP HTTP endpoint")
    }
}

/// Builds an LLM-safe tool name: `<server>__<tool>` with only [a-zA-Z0-9_-].
pub fn prefixed_tool_name(server_name: &str, tool: &str) -> String {
    let sanitize = |s: &str| -> String {
        s.chars()
            .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
            .collect()
    };
    format!("{}__{}", sanitize(server_name), sanitize(tool))
}
