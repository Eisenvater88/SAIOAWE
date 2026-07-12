use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

/// One chat message in Ollama's /api/chat format.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    #[serde(default)]
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    /// Name of the tool a `role: "tool"` message answers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
}

impl ChatMessage {
    pub fn new(role: &str, content: impl Into<String>) -> Self {
        Self {
            role: role.into(),
            content: content.into(),
            tool_calls: None,
            tool_name: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub function: ToolCallFunction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallFunction {
    pub name: String,
    #[serde(default)]
    pub arguments: Value,
}

/// Tool definition in the OpenAI-style format Ollama expects.
#[derive(Debug, Clone, Serialize)]
pub struct ToolDef {
    pub r#type: String,
    pub function: ToolFunctionDef,
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolFunctionDef {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

impl ToolDef {
    pub fn new(name: String, description: String, parameters: Value) -> Self {
        Self {
            r#type: "function".into(),
            function: ToolFunctionDef {
                name,
                description,
                parameters,
            },
        }
    }
}

#[derive(Debug, Deserialize)]
struct ChatResponse {
    message: ChatMessage,
}

#[derive(Debug, Deserialize)]
struct TagsResponse {
    #[serde(default)]
    models: Vec<TagModel>,
}

#[derive(Debug, Deserialize)]
struct TagModel {
    name: String,
}

pub struct OllamaClient {
    http: reqwest::Client,
    base_url: String,
}

impl OllamaClient {
    pub fn new(base_url: &str, timeout_secs: u64) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(timeout_secs))
            .build()
            .expect("reqwest client");
        Self {
            http,
            base_url: base_url.trim_end_matches('/').to_string(),
        }
    }

    /// Single non-streaming chat completion, optionally with tools.
    pub async fn chat(
        &self,
        model: &str,
        messages: &[ChatMessage],
        tools: &[ToolDef],
        temperature: f32,
    ) -> Result<ChatMessage> {
        let mut body = serde_json::json!({
            "model": model,
            "messages": messages,
            "stream": false,
            "options": { "temperature": temperature },
        });
        if !tools.is_empty() {
            body["tools"] = serde_json::to_value(tools)?;
        }
        let resp = self
            .http
            .post(format!("{}/api/chat", self.base_url))
            .json(&body)
            .send()
            .await
            .context("request to Ollama failed - is Ollama running?")?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(anyhow!("Ollama returned {status}: {text}"));
        }
        let parsed: ChatResponse =
            serde_json::from_str(&text).context("unexpected response from Ollama /api/chat")?;
        Ok(parsed.message)
    }

    /// List locally available models (GET /api/tags).
    pub async fn list_models(&self) -> Result<Vec<String>> {
        let resp = self
            .http
            .get(format!("{}/api/tags", self.base_url))
            .timeout(Duration::from_secs(5))
            .send()
            .await?
            .error_for_status()?;
        let tags: TagsResponse = resp.json().await?;
        Ok(tags.models.into_iter().map(|m| m.name).collect())
    }
}
