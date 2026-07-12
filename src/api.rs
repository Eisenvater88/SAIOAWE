use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use futures::Stream;
use futures::StreamExt;
use serde::Deserialize;
use serde_json::{json, Value};
use std::convert::Infallible;
use std::sync::Arc;
use tokio_stream::wrappers::BroadcastStream;
use tower_http::cors::CorsLayer;
use tower_http::services::{ServeDir, ServeFile};

use crate::config::Config;
use crate::engine::{validate_graph, Engine};
use crate::models::*;
use crate::scheduler::normalize_cron;

pub struct AppState {
    pub cfg: Config,
    pub engine: Arc<Engine>,
}

type St = State<Arc<AppState>>;

/// anyhow -> HTTP 500 with a JSON error body.
pub struct ApiError(anyhow::Error, StatusCode);

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        (self.1, Json(json!({ "error": format!("{:#}", self.0) }))).into_response()
    }
}

impl<E: Into<anyhow::Error>> From<E> for ApiError {
    fn from(e: E) -> Self {
        Self(e.into(), StatusCode::INTERNAL_SERVER_ERROR)
    }
}

fn bad_request(msg: impl Into<String>) -> ApiError {
    ApiError(anyhow::anyhow!(msg.into()), StatusCode::BAD_REQUEST)
}

fn not_found() -> ApiError {
    ApiError(anyhow::anyhow!("not found"), StatusCode::NOT_FOUND)
}

type ApiResult<T> = Result<Json<T>, ApiError>;

pub fn router(state: Arc<AppState>) -> Router {
    let web_dir = state.cfg.web_dir.clone();
    let static_files =
        ServeDir::new(&web_dir).not_found_service(ServeFile::new(web_dir.join("index.html")));
    Router::new()
        .route("/api/config", get(get_config))
        .route("/api/agents", get(list_agents).post(create_agent))
        .route(
            "/api/agents/{id}",
            get(get_agent).put(update_agent).delete(delete_agent),
        )
        .route("/api/workflows", get(list_workflows).post(create_workflow))
        .route(
            "/api/workflows/{id}",
            get(get_workflow).put(update_workflow).delete(delete_workflow),
        )
        .route("/api/workflows/{id}/run", post(run_workflow))
        .route("/api/mcp-servers", get(list_mcp).post(create_mcp))
        .route(
            "/api/mcp-servers/{id}",
            get(get_mcp).put(update_mcp).delete(delete_mcp),
        )
        .route("/api/mcp-servers/{id}/tools", get(mcp_tools))
        .route("/api/schedules", get(list_schedules).post(create_schedule))
        .route(
            "/api/schedules/{id}",
            get(get_schedule).put(update_schedule).delete(delete_schedule),
        )
        .route("/api/runs", get(list_runs))
        .route("/api/runs/{id}", get(get_run))
        .route("/api/runs/{id}/cancel", post(cancel_run))
        .route("/api/events", get(sse_events))
        .fallback_service(static_files)
        .layer(CorsLayer::permissive())
        .with_state(state)
}

// ---------------------------------------------------------------- config

async fn get_config(State(st): St) -> ApiResult<Value> {
    let models = st.engine.ollama.list_models().await.unwrap_or_default();
    Ok(Json(json!({
        "ollama_url": st.cfg.ollama_url,
        "default_model": st.cfg.ollama_model,
        "default_temperature": st.cfg.temperature,
        "models": models,
    })))
}

// ---------------------------------------------------------------- agents

async fn list_agents(State(st): St) -> ApiResult<Vec<AgentCard>> {
    let mut agents: Vec<AgentCard> = st.engine.db.list("agent_cards")?;
    agents.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(Json(agents))
}

async fn create_agent(State(st): St, Json(mut card): Json<AgentCard>) -> ApiResult<AgentCard> {
    if card.name.trim().is_empty() {
        return Err(bad_request("agent name must not be empty"));
    }
    card.id = new_id();
    card.created_at = now_rfc3339();
    card.updated_at = card.created_at.clone();
    st.engine.db.put("agent_cards", &card.id.clone(), &card)?;
    Ok(Json(card))
}

async fn get_agent(State(st): St, Path(id): Path<String>) -> ApiResult<AgentCard> {
    st.engine
        .db
        .get("agent_cards", &id)?
        .map(Json)
        .ok_or_else(not_found)
}

async fn update_agent(
    State(st): St,
    Path(id): Path<String>,
    Json(mut card): Json<AgentCard>,
) -> ApiResult<AgentCard> {
    let existing: AgentCard = st.engine.db.get("agent_cards", &id)?.ok_or_else(not_found)?;
    card.id = id.clone();
    card.created_at = existing.created_at;
    card.updated_at = now_rfc3339();
    st.engine.db.put("agent_cards", &id, &card)?;
    Ok(Json(card))
}

async fn delete_agent(State(st): St, Path(id): Path<String>) -> ApiResult<Value> {
    st.engine.db.delete("agent_cards", &id)?;
    Ok(Json(json!({ "deleted": true })))
}

// ---------------------------------------------------------------- workflows

async fn list_workflows(State(st): St) -> ApiResult<Vec<Workflow>> {
    let mut wfs: Vec<Workflow> = st.engine.db.list("workflows")?;
    wfs.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(Json(wfs))
}

async fn create_workflow(State(st): St, Json(mut wf): Json<Workflow>) -> ApiResult<Workflow> {
    if wf.name.trim().is_empty() {
        return Err(bad_request("workflow name must not be empty"));
    }
    validate_graph(&wf.graph).map_err(|e| bad_request(format!("{e:#}")))?;
    wf.id = new_id();
    wf.created_at = now_rfc3339();
    wf.updated_at = wf.created_at.clone();
    st.engine.db.put("workflows", &wf.id.clone(), &wf)?;
    Ok(Json(wf))
}

async fn get_workflow(State(st): St, Path(id): Path<String>) -> ApiResult<Workflow> {
    st.engine
        .db
        .get("workflows", &id)?
        .map(Json)
        .ok_or_else(not_found)
}

async fn update_workflow(
    State(st): St,
    Path(id): Path<String>,
    Json(mut wf): Json<Workflow>,
) -> ApiResult<Workflow> {
    let existing: Workflow = st.engine.db.get("workflows", &id)?.ok_or_else(not_found)?;
    validate_graph(&wf.graph).map_err(|e| bad_request(format!("{e:#}")))?;
    wf.id = id.clone();
    wf.created_at = existing.created_at;
    wf.updated_at = now_rfc3339();
    st.engine.db.put("workflows", &id, &wf)?;
    Ok(Json(wf))
}

async fn delete_workflow(State(st): St, Path(id): Path<String>) -> ApiResult<Value> {
    st.engine.db.delete("workflows", &id)?;
    // Remove schedules pointing at the deleted workflow.
    let schedules: Vec<Schedule> = st.engine.db.list("schedules")?;
    for s in schedules.iter().filter(|s| s.workflow_id == id) {
        st.engine.db.delete("schedules", &s.id)?;
    }
    Ok(Json(json!({ "deleted": true })))
}

#[derive(Deserialize, Default)]
struct RunBody {
    #[serde(default)]
    input: String,
}

async fn run_workflow(
    State(st): St,
    Path(id): Path<String>,
    body: Option<Json<RunBody>>,
) -> ApiResult<Run> {
    let input = body.map(|b| b.0.input).unwrap_or_default();
    let run = st
        .engine
        .start_run(&id, "manual", input)
        .map_err(|e| bad_request(format!("{e:#}")))?;
    Ok(Json(run))
}

// ---------------------------------------------------------------- mcp servers

async fn list_mcp(State(st): St) -> ApiResult<Vec<McpServerConfig>> {
    let mut servers: Vec<McpServerConfig> = st.engine.db.list("mcp_servers")?;
    servers.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(Json(servers))
}

async fn create_mcp(State(st): St, Json(mut cfg): Json<McpServerConfig>) -> ApiResult<McpServerConfig> {
    if cfg.name.trim().is_empty() {
        return Err(bad_request("MCP server name must not be empty"));
    }
    cfg.id = new_id();
    cfg.created_at = now_rfc3339();
    cfg.updated_at = cfg.created_at.clone();
    st.engine.db.put("mcp_servers", &cfg.id.clone(), &cfg)?;
    Ok(Json(cfg))
}

async fn get_mcp(State(st): St, Path(id): Path<String>) -> ApiResult<McpServerConfig> {
    st.engine
        .db
        .get("mcp_servers", &id)?
        .map(Json)
        .ok_or_else(not_found)
}

async fn update_mcp(
    State(st): St,
    Path(id): Path<String>,
    Json(mut cfg): Json<McpServerConfig>,
) -> ApiResult<McpServerConfig> {
    let existing: McpServerConfig = st.engine.db.get("mcp_servers", &id)?.ok_or_else(not_found)?;
    cfg.id = id.clone();
    cfg.created_at = existing.created_at;
    cfg.updated_at = now_rfc3339();
    st.engine.db.put("mcp_servers", &id, &cfg)?;
    st.engine.mcp.disconnect(&id).await; // force reconnect with new settings
    Ok(Json(cfg))
}

async fn delete_mcp(State(st): St, Path(id): Path<String>) -> ApiResult<Value> {
    st.engine.mcp.disconnect(&id).await;
    st.engine.db.delete("mcp_servers", &id)?;
    Ok(Json(json!({ "deleted": true })))
}

/// Connects (if needed) and lists the server's tools - doubles as a
/// connection test for the UI.
async fn mcp_tools(State(st): St, Path(id): Path<String>) -> ApiResult<Value> {
    let cfg: McpServerConfig = st.engine.db.get("mcp_servers", &id)?.ok_or_else(not_found)?;
    let tools = st
        .engine
        .mcp
        .list_tools(&cfg)
        .await
        .map_err(|e| bad_request(format!("{e:#}")))?;
    Ok(Json(json!({ "tools": tools })))
}

// ---------------------------------------------------------------- schedules

async fn list_schedules(State(st): St) -> ApiResult<Vec<Schedule>> {
    Ok(Json(st.engine.db.list("schedules")?))
}

async fn create_schedule(State(st): St, Json(mut s): Json<Schedule>) -> ApiResult<Schedule> {
    s.cron = normalize_cron(&s.cron).map_err(|e| bad_request(format!("{e:#}")))?;
    let wf: Option<Workflow> = st.engine.db.get("workflows", &s.workflow_id)?;
    if wf.is_none() {
        return Err(bad_request("schedule references an unknown workflow"));
    }
    s.id = new_id();
    s.created_at = now_rfc3339();
    st.engine.db.put("schedules", &s.id.clone(), &s)?;
    Ok(Json(s))
}

async fn get_schedule(State(st): St, Path(id): Path<String>) -> ApiResult<Schedule> {
    st.engine
        .db
        .get("schedules", &id)?
        .map(Json)
        .ok_or_else(not_found)
}

async fn update_schedule(
    State(st): St,
    Path(id): Path<String>,
    Json(mut s): Json<Schedule>,
) -> ApiResult<Schedule> {
    let existing: Schedule = st.engine.db.get("schedules", &id)?.ok_or_else(not_found)?;
    s.cron = normalize_cron(&s.cron).map_err(|e| bad_request(format!("{e:#}")))?;
    s.id = id.clone();
    s.created_at = existing.created_at;
    st.engine.db.put("schedules", &id, &s)?;
    Ok(Json(s))
}

async fn delete_schedule(State(st): St, Path(id): Path<String>) -> ApiResult<Value> {
    st.engine.db.delete("schedules", &id)?;
    Ok(Json(json!({ "deleted": true })))
}

// ---------------------------------------------------------------- runs

#[derive(Deserialize)]
struct RunsQuery {
    workflow_id: Option<String>,
    limit: Option<u32>,
}

async fn list_runs(State(st): St, Query(q): Query<RunsQuery>) -> ApiResult<Vec<Run>> {
    Ok(Json(st.engine.db.list_runs(
        q.workflow_id.as_deref(),
        q.limit.unwrap_or(50).min(500),
    )?))
}

async fn get_run(State(st): St, Path(id): Path<String>) -> ApiResult<Value> {
    let run = st.engine.db.get_run(&id)?.ok_or_else(not_found)?;
    let node_runs = st.engine.db.node_runs_for(&id)?;
    Ok(Json(json!({ "run": run, "node_runs": node_runs })))
}

async fn cancel_run(State(st): St, Path(id): Path<String>) -> ApiResult<Value> {
    st.engine.cancel(&id);
    Ok(Json(json!({ "canceling": true })))
}

// ---------------------------------------------------------------- events

async fn sse_events(State(st): St) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let rx = st.engine.events.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|item| async move {
        let ev = item.ok()?;
        let data = serde_json::to_string(&ev).ok()?;
        Some(Ok(Event::default().event("run").data(data)))
    });
    Sse::new(stream).keep_alive(KeepAlive::default())
}
