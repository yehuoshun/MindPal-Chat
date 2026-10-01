use crate::config::AppConfig;
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
    stream_chat_with(client, cfg, system_prompt, messages, |delta| {
        let _ = app.emit("llm-token", delta);
    })
    .await
}

/// 流式聊天主逻辑（不依赖 Tauri 句柄，可用 mock 服务端做集成测试）
pub async fn stream_chat_with(
    client: &Client,
    cfg: &AppConfig,
    system_prompt: &str,
    messages: &[ChatMessage],
    on_token: impl FnMut(&str),
) -> Result<String, String> {
    match cfg.provider.as_str() {
        "claude" => claude_chat(client, cfg, system_prompt, messages, on_token).await,
        _ => openai_chat(client, cfg, system_prompt, messages, on_token).await,
    }
}

/// OpenAI 兼容协议（DeepSeek / 大多数国内模型）
async fn openai_chat(
    client: &Client,
    cfg: &AppConfig,
    system_prompt: &str,
    messages: &[ChatMessage],
    on_token: impl FnMut(&str),
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
    consume_sse_openai(resp, on_token).await
}

/// OpenAI 兼容 SSE 消费（纯逻辑，可测）：逐块拼文本，[DONE] 提前结束
async fn consume_sse_openai(
    resp: reqwest::Response,
    mut on_token: impl FnMut(&str),
) -> Result<String, String> {
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
                    on_token(delta);
                }
            }
        }
    }
    Ok(full)
}

/// Anthropic Claude：x-api-key 头 + system 字段独立 + SSE 事件类型不同
async fn claude_chat(
    client: &Client,
    cfg: &AppConfig,
    system_prompt: &str,
    messages: &[ChatMessage],
    on_token: impl FnMut(&str),
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
    consume_sse_claude(resp, on_token).await
}

/// Claude SSE 消费（纯逻辑，可测）：content_block_delta 事件取 delta.text
async fn consume_sse_claude(
    resp: reqwest::Response,
    mut on_token: impl FnMut(&str),
) -> Result<String, String> {
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
                        on_token(text);
                    }
                }
            }
        }
    }
    Ok(full)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    /// 起一个本地 SSE mock 服务器，返回 base url（服务一个连接后退出）
    fn spawn_sse_server(body: &'static str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            if let Ok(mut stream) = listener.accept().map(|(s, _)| s) {
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf);
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(resp.as_bytes());
            }
        });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn consume_openai_sse_from_mock_server() {
        let body = "data: {\"choices\":[{\"delta\":{\"content\":\"你\"}}]}\n\n\
                    data: {\"choices\":[{\"delta\":{\"content\":\"好\"}}]}\n\n\
                    data: {\"choices\":[{\"delta\":{\"content\":\"！\"}}]}\n\n\
                    data: [DONE]\n\n";
        let url = spawn_sse_server(body);
        let client = Client::new();
        let resp = client.get(&url).send().await.unwrap();
        let mut tokens: Vec<String> = Vec::new();
        let full = consume_sse_openai(resp, |t| tokens.push(t.to_string()))
            .await
            .unwrap();
        assert_eq!(full, "你好！");
        assert_eq!(tokens, vec!["你", "好", "！"]);
    }

    #[tokio::test]
    async fn consume_claude_sse_from_mock_server() {
        let body = "event: content_block_delta\n\
                    data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"哈\"}}\n\n\
                    event: content_block_delta\n\
                    data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"喽\"}}\n\n\
                    event: message_stop\n\
                    data: {\"type\":\"message_stop\"}\n\n";
        let url = spawn_sse_server(body);
        let client = Client::new();
        let resp = client.get(&url).send().await.unwrap();
        let mut tokens: Vec<String> = Vec::new();
        let full = consume_sse_claude(resp, |t| tokens.push(t.to_string()))
            .await
            .unwrap();
        assert_eq!(full, "哈喽");
        assert_eq!(tokens, vec!["哈", "喽"]);
    }

    #[tokio::test]
    async fn consume_openai_http_error() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            if let Ok(mut stream) = listener.accept().map(|(s, _)| s) {
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf);
                let resp = "HTTP/1.1 500 Internal Server Error\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}";
                let _ = stream.write_all(resp.as_bytes());
            }
        });
        let client = Client::new();
        // 直接消费 500 响应：SSE 解析应正常结束（空文本），错误处理在调用方 status 检查
        let resp = client.get(format!("http://{addr}")).send().await.unwrap();
        assert_eq!(resp.status().as_u16(), 500);
        let full = consume_sse_openai(resp, |_| {}).await.unwrap();
        assert!(full.is_empty());
    }

    // ---------- 请求级集成测试：mock 服务端抓原始请求 ----------

    /// 完整读取一个 HTTP 请求（头部 + Content-Length 指定长度）
    fn read_full_request(stream: &mut std::net::TcpStream) -> String {
        let mut data: Vec<u8> = Vec::new();
        let mut buf = [0u8; 4096];
        loop {
            match stream.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    data.extend_from_slice(&buf[..n]);
                    let text = String::from_utf8_lossy(&data).to_string();
                    if let Some(pos) = text.find("\r\n\r\n") {
                        let clen = text[..pos]
                            .lines()
                            .find_map(|l| {
                                l.to_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                            })
                            .unwrap_or(0);
                        if data.len() >= pos + 4 + clen {
                            break;
                        }
                    }
                }
                Err(_) => break,
            }
        }
        String::from_utf8_lossy(&data).to_string()
    }

    /// 起一个记录原始请求的 mock 服务器，返回 (base_url, 捕获的请求)
    fn spawn_capturing_server(
        body: &'static str,
    ) -> (String, std::sync::Arc<std::sync::Mutex<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let captured = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let sink = captured.clone();
        std::thread::spawn(move || {
            if let Ok(mut stream) = listener.accept().map(|(s, _)| s) {
                *sink.lock().unwrap() = read_full_request(&mut stream);
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(resp.as_bytes());
            }
        });
        (format!("http://{addr}"), captured)
    }

    fn test_cfg(provider: &str, base_url: String) -> AppConfig {
        AppConfig {
            provider: provider.to_string(),
            api_key: "sk-test-key".to_string(),
            model: "test-model".to_string(),
            base_url,
            ..AppConfig::default()
        }
    }

    #[tokio::test]
    async fn openai_chat_end_to_end_against_mock() {
        let body = "data: {\"choices\":[{\"delta\":{\"content\":\"流\"}}]}\n\n\
                    data: {\"choices\":[{\"delta\":{\"content\":\"式\"}}]}\n\n\
                    data: [DONE]\n\n";
        let (url, captured) = spawn_capturing_server(body);
        let cfg = test_cfg("deepseek", url);
        let messages = vec![ChatMessage {
            role: "user".into(),
            content: "在吗".into(),
        }];
        let mut tokens: Vec<String> = Vec::new();
        let full = stream_chat_with(&Client::new(), &cfg, "你是北极熊", &messages, |t| {
            tokens.push(t.to_string())
        })
        .await
        .unwrap();
        assert_eq!(full, "流式");
        assert_eq!(tokens, vec!["流", "式"]);

        let req = captured.lock().unwrap().clone();
        let lower = req.to_lowercase();
        assert!(req.starts_with("POST /chat/completions"), "req={req}");
        assert!(lower.contains("authorization: bearer sk-test-key"), "req={req}");
        assert!(req.contains("\"stream\":true"), "req={req}");
        assert!(req.contains("test-model"), "req={req}");
        assert!(req.contains("你是北极熊"), "req={req}"); // system prompt 注入
        assert!(req.contains("在吗"), "req={req}");
        assert!(req.contains("\"role\":\"system\""), "req={req}");
    }

    #[tokio::test]
    async fn claude_chat_end_to_end_against_mock() {
        let body = "event: content_block_delta\n\
                    data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"噦\"}}\n\n\
                    event: message_stop\n\
                    data: {\"type\":\"message_stop\"}\n\n";
        let (url, captured) = spawn_capturing_server(body);
        let cfg = test_cfg("claude", url);
        let messages = vec![ChatMessage {
            role: "user".into(),
            content: "hi".into(),
        }];
        let mut tokens: Vec<String> = Vec::new();
        let full = stream_chat_with(&Client::new(), &cfg, "sys-prompt", &messages, |t| {
            tokens.push(t.to_string())
        })
        .await
        .unwrap();
        assert_eq!(full, "噦");
        assert_eq!(tokens, vec!["噦"]);

        let req = captured.lock().unwrap().clone();
        let lower = req.to_lowercase();
        assert!(req.starts_with("POST /messages"), "req={req}");
        assert!(lower.contains("x-api-key: sk-test-key"), "req={req}");
        assert!(lower.contains("anthropic-version: 2023-06-01"), "req={req}");
        assert!(req.contains("\"system\":\"sys-prompt\""), "req={req}");
    }

    #[tokio::test]
    async fn chat_surfaces_api_error_status() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            if let Ok(mut stream) = listener.accept().map(|(s, _)| s) {
                let _ = read_full_request(&mut stream);
                let _ = stream.write_all(
                    b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 11\r\nConnection: close\r\n\r\n{\"e\":\"bad\"}",
                );
            }
        });
        let cfg = test_cfg("deepseek", format!("http://{addr}"));
        let messages = vec![ChatMessage {
            role: "user".into(),
            content: "x".into(),
        }];
        let err = stream_chat_with(&Client::new(), &cfg, "", &messages, |_| {})
            .await
            .unwrap_err();
        assert!(err.contains("401"), "err={err}");
    }
}
