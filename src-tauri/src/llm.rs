use crate::config::{AppConfig, CLAUDE_BASE, CLAUDE_MODEL, DEEPSEEK_BASE, DEEPSEEK_MODEL};
use crate::types::ChatMessage;
use futures_util::StreamExt;
use reqwest::Client;
use serde_json::json;
use tauri::{AppHandle, Emitter};

/// LLM 路由入口：按 provider 分流，流式返回完整文本
/// 过程中通过 `llm-token` 事件逐段推送增量文本
pub async fn stream_chat(
    app: &AppHandle,
    client: &Client,
    cfg: &AppConfig,
    system_prompt: &str,
    messages: &[ChatMessage],
) -> Result<String, String> {
    match cfg.provider.as_str() {
        "claude" => stream_claude(app, client, cfg, system_prompt, messages).await,
        _ => stream_openai_compatible(app, client, cfg, system_prompt, messages).await,
    }
}

/// OpenAI 兼容协议（DeepSeek / 大多数国内模型）
async fn stream_openai_compatible(
    app: &AppHandle,
    client: &Client,
    cfg: &AppConfig,
    system_prompt: &str,
    messages: &[ChatMessage],
) -> Result<String, String> {
    let mut api_messages: Vec<serde_json::Value> = Vec::new();
    if !system_prompt.trim().is_empty() {
        api_messages.push(json!({ "role": "system", "content": system_prompt }));
    }
    for m in messages {
        api_messages.push(json!({ "role": m.role, "content": m.content }));
    }
    let body = json!({
        "model": cfg.model,
        "messages": api_messages,
        "stream": true,
        "temperature": cfg.temperature
    });
    let url = format!("{}/chat/completions", cfg.base_url);

    let resp = client
        .post(&url)
        .bearer_auth(&cfg.api_key)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("请求 LLM 失败: {e}"))?;
    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("LLM API 错误 {status}: {text}"));
    }
    sse_loop_openai(app, resp).await
}

/// 解析 OpenAI 兼容 SSE：`data: {json}` / `data: [DONE]`
async fn sse_loop_openai(app: &AppHandle, resp: reqwest::Response) -> Result<String, String> {
    let mut buf = String::new();
    let mut full = String::new();
    let mut stream = resp.bytes_stream();
    while let Some(item) = stream.next().await {
        let bytes = item.map_err(|e| format!("读取流失败: {e}"))?;
        buf.push_str(&String::from_utf8_lossy(&bytes));
        while let Some(pos) = buf.find('\n') {
            let line: String = buf[..pos].trim().to_string();
            buf.drain(..=pos);
            let Some(data) = line.strip_prefix("data:") else {
                continue;
            };
            let data = data.trim();
            if data == "[DONE]" {
                return Ok(full);
            }
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(data) {
                if let Some(delta) = v["choices"][0]["delta"]["content"].as_str() {
                    full.push_str(delta);
                    let _ = app.emit("llm-token", delta);
                }
            }
        }
    }
    Ok(full)
}

/// Anthropic Claude：x-api-key 头 + system 字段独立 + SSE 事件类型不同
async fn stream_claude(
    app: &AppHandle,
    client: &Client,
    cfg: &AppConfig,
    system_prompt: &str,
    messages: &[ChatMessage],
) -> Result<String, String> {
    let api_messages: Vec<serde_json::Value> = messages
        .iter()
        .filter(|m| m.role != "system")
        .map(|m| json!({ "role": m.role, "content": m.content }))
        .collect();
    let body = json!({
        "model": cfg.model,
        "system": system_prompt,
        "messages": api_messages,
        "stream": true,
        "max_tokens": 4096
    });
    let url = format!("{}/messages", cfg.base_url);

    let resp = client
        .post(&url)
        .header("x-api-key", &cfg.api_key)
        .header("anthropic-version", "2023-06-01")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("请求 LLM 失败: {e}"))?;
    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("LLM API 错误 {status}: {text}"));
    }
    sse_loop_claude(app, resp).await
}

/// 解析 Claude SSE：content_block_delta 事件里的 delta.text
async fn sse_loop_claude(app: &AppHandle, resp: reqwest::Response) -> Result<String, String> {
    let mut buf = String::new();
    let mut full = String::new();
    let mut stream = resp.bytes_stream();
    while let Some(item) = stream.next().await {
        let bytes = item.map_err(|e| format!("读取流失败: {e}"))?;
        buf.push_str(&String::from_utf8_lossy(&bytes));
        while let Some(pos) = buf.find('\n') {
            let line: String = buf[..pos].trim().to_string();
            buf.drain(..=pos);
            let Some(data) = line.strip_prefix("data:") else {
                continue;
            };
            let data = data.trim();
            if data.is_empty() || data == "[DONE]" {
                continue;
            }
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(data) {
                if v["type"].as_str() == Some("content_block_delta") {
                    if let Some(text) = v["delta"]["text"].as_str() {
                        full.push_str(text);
                        let _ = app.emit("llm-token", text);
                    }
                }
            }
        }
    }
    Ok(full)
}

/// 按 provider 给默认端点/模型（前端切换 provider 时自动填充用）
pub fn defaults_for(provider: &str) -> (String, String) {
    match provider {
        "claude" => (CLAUDE_BASE.to_string(), CLAUDE_MODEL.to_string()),
        _ => (DEEPSEEK_BASE.to_string(), DEEPSEEK_MODEL.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_defaults() {
        let (base, model) = defaults_for("deepseek");
        assert_eq!(base, DEEPSEEK_BASE);
        assert_eq!(model, DEEPSEEK_MODEL);
        let (base, model) = defaults_for("claude");
        assert_eq!(base, CLAUDE_BASE);
        assert_eq!(model, CLAUDE_MODEL);
        // 未知 provider 回退 DeepSeek
        let (base, _) = defaults_for("unknown");
        assert_eq!(base, DEEPSEEK_BASE);
    }
}
