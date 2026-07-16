mod api;
mod config;
mod db;
mod engine;
mod mcp;
mod models;
mod ollama;
mod scheduler;

use anyhow::Result;
use clap::Parser;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;

use crate::api::AppState;
use crate::config::Config;
use crate::db::Db;
use crate::engine::Engine;
use crate::mcp::McpManager;
use crate::ollama::OllamaClient;

#[tokio::main]
async fn main() -> Result<()> {
    let cfg = Config::parse();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "saioawe=info,tower_http=info".into()),
        )
        .init();

    let db = Arc::new(Db::open(&cfg.db)?);
    match db.reconcile_interrupted() {
        Ok(0) => {}
        Ok(n) => tracing::warn!("marked {n} run(s) interrupted (left running by a previous restart)"),
        Err(e) => tracing::error!("run reconciliation failed: {e:#}"),
    }
    let ollama = Arc::new(OllamaClient::new(&cfg.ollama_url, cfg.llm_timeout));
    let mcp = Arc::new(McpManager::new(Duration::from_secs(cfg.tool_timeout)));
    let (events, _) = broadcast::channel(1024);

    let engine = Arc::new(Engine::new(
        db,
        ollama,
        mcp,
        events,
        cfg.ollama_model.clone(),
        cfg.temperature,
    ));

    tokio::spawn(scheduler::run_scheduler(engine.clone()));

    let state = Arc::new(AppState {
        cfg: cfg.clone(),
        engine,
    });
    let app = api::router(state);

    let addr = format!("{}:{}", cfg.host, cfg.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("SAIOAWE listening on http://{addr} (Ollama: {})", cfg.ollama_url);
    axum::serve(listener, app).await?;
    Ok(())
}
