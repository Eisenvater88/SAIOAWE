use clap::Parser;
use std::path::PathBuf;

/// SAIOAWE - agent workflow orchestrator.
///
/// Agents are described by agent cards, wired into directed workflows,
/// executed against a local Ollama instance and given tools via MCP servers.
#[derive(Parser, Debug, Clone)]
#[command(name = "saioawe", version, about)]
pub struct Config {
    /// Address to bind the web server to
    #[arg(long, env = "SAIOAWE_HOST", default_value = "127.0.0.1")]
    pub host: String,

    /// Port for the web UI / API
    #[arg(long, env = "SAIOAWE_PORT", default_value_t = 8321)]
    pub port: u16,

    /// Base URL of the Ollama server
    #[arg(long, env = "OLLAMA_URL", default_value = "http://127.0.0.1:11434")]
    pub ollama_url: String,

    /// Default model used when an agent card does not specify one
    #[arg(long, env = "OLLAMA_MODEL", default_value = "llama3.1")]
    pub ollama_model: String,

    /// Default sampling temperature for agents that do not set one
    #[arg(long, env = "OLLAMA_TEMPERATURE", default_value_t = 0.7)]
    pub temperature: f32,

    /// Path to the SQLite database file
    #[arg(long, env = "SAIOAWE_DB", default_value = "saioawe.db")]
    pub db: PathBuf,

    /// Directory containing the built web UI (served statically)
    #[arg(long, env = "SAIOAWE_WEB_DIR", default_value = "web/dist")]
    pub web_dir: PathBuf,

    /// Timeout in seconds for a single LLM request
    #[arg(long, env = "SAIOAWE_LLM_TIMEOUT", default_value_t = 600)]
    pub llm_timeout: u64,

    /// Timeout in seconds for a single MCP tool call
    #[arg(long, env = "SAIOAWE_TOOL_TIMEOUT", default_value_t = 120)]
    pub tool_timeout: u64,
}
