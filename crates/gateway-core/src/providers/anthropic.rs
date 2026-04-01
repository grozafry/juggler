use super::Provider;
use crate::server::models::{ChatCompletionChunk, ChatCompletionRequest, ChunkChoice, Delta};
use axum::response::sse::Event;
use futures::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::convert::Infallible;
use std::env;
use std::pin::Pin;
use tokio_stream::Stream;
use uuid::Uuid;

pub struct AnthropicProvider;

#[derive(Serialize)]
struct AnthropicMessage {
    role: String,
    content: String,
}

#[derive(Serialize)]
struct AnthropicRequest {
    model: String,
    messages: Vec<AnthropicMessage>,
    system: Option<String>,
    max_tokens: u32,
    stream: bool,
}

#[allow(dead_code)]
#[derive(Deserialize, Debug)]
#[serde(tag = "type")]
enum AnthropicEvent {
    #[serde(rename = "message_start")]
    MessageStart { message: AnthropicMessageObj },
    #[serde(rename = "content_block_start")]
    ContentBlockStart { index: u32 },
    #[serde(rename = "ping")]
    Ping {},
    #[serde(rename = "content_block_delta")]
    ContentBlockDelta { index: u32, delta: AnthropicTextDelta },
    #[serde(rename = "content_block_stop")]
    ContentBlockStop { index: u32 },
    #[serde(rename = "message_delta")]
    MessageDelta { delta: AnthropicMessageDelta, usage: Option<AnthropicUsage> },
    #[serde(rename = "message_stop")]
    MessageStop {},
    #[serde(rename = "error")]
    Error { error: AnthropicError },
}

#[allow(dead_code)]
#[derive(Deserialize, Debug)]
struct AnthropicMessageObj {
    id: String,
    usage: Option<AnthropicUsage>,
}

#[allow(dead_code)]
#[derive(Deserialize, Debug)]
struct AnthropicUsage {
    input_tokens: Option<u32>,
    output_tokens: Option<u32>,
}

#[allow(dead_code)]
#[derive(Deserialize, Debug)]
struct AnthropicTextDelta {
    #[serde(rename = "type")]
    delta_type: String,
    text: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize, Debug)]
struct AnthropicMessageDelta {
    stop_reason: Option<String>,
    usage: Option<AnthropicUsage>,   // final output_tokens count
}

#[derive(Deserialize, Debug)]
struct AnthropicError {
    message: String,
}

impl Provider for AnthropicProvider {
    fn stream(
        &self,
        client: Client,
        model: &str,
        request: ChatCompletionRequest,
        breaker: std::sync::Arc<crate::reliability::breaker::CircuitBreaker>,
        request_id: String,
        workspace_id: String,
        virtual_key_id: String,
        audit_publisher: std::sync::Arc<crate::reliability::audit::AuditPublisher>,
    ) -> Pin<Box<dyn Stream<Item = Result<Event, Infallible>> + Send>> {
        let api_key = env::var("ANTHROPIC_API_KEY").unwrap_or_default();
        if api_key.is_empty() {
            tracing::warn!("ANTHROPIC_API_KEY is not set in the environment");
        }

        let mut sys_prompt = None;
        let mut anthropic_messages = Vec::new();

        for m in request.messages {
            if m.role == "system" {
                sys_prompt = Some(m.content);
            } else {
                anthropic_messages.push(AnthropicMessage {
                    role: m.role,
                    content: m.content,
                });
            }
        }

        let anthropic_req = AnthropicRequest {
            model: model.to_string(),
            messages: anthropic_messages,
            system: sys_prompt,
            max_tokens: request.max_tokens.unwrap_or(1024),
            stream: true,
        };

        let req_id = Uuid::new_v4().to_string();
        let model_name = model.to_string();

        let stream = async_stream::stream! {
            tracing::info!("Connecting to Anthropic API: {}", model_name);
            let started_at = std::time::Instant::now();
            let mut prompt_tokens: u32 = 0;
            let mut completion_tokens: u32 = 0;
            let mut ttfb_ms: u32 = 0;
            let mut error_code = String::new();
            let mut http_start = std::time::Instant::now();

            let mut attempts = 0;
            let max_attempts = 3;
            let res;

            loop {
                http_start = std::time::Instant::now();
                let attempt_res = client.post("https://api.anthropic.com/v1/messages")
                    .header("x-api-key", &api_key)
                    .header("anthropic-version", "2023-06-01")
                    .header("x-request-id", request_id.clone())
                    .json(&anthropic_req)
                    .send()
                    .await;

                match &attempt_res {
                    Ok(r) if r.status() == 429 || r.status() == 503 || r.status() == 524 => {
                        if attempts >= max_attempts - 1 {
                            res = attempt_res;
                            break;
                        }
                        attempts += 1;
                        let base_delay = std::time::Duration::from_millis(500);
                        let max_delay = std::time::Duration::from_millis(5000);
                        let exponential = std::cmp::min(max_delay, base_delay * 2_u32.pow(attempts));
                        let jitter = std::time::Duration::from_millis(rand::random::<u64>() % base_delay.as_millis() as u64);
                        tokio::time::sleep(exponential + jitter).await;
                    }
                    Err(_) => {
                        if attempts >= max_attempts - 1 {
                            res = attempt_res;
                            break;
                        }
                        attempts += 1;
                        tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
                    }
                    _ => {
                        res = attempt_res;
                        break;
                    }
                }
            }

            match res {
                Ok(response) => {
                    let status = response.status();
                    if !status.is_success() {
                        breaker.record_failure();
                        let err_text = response.text().await.unwrap_or_default();
                        tracing::error!("Anthropic API returned error {}: {}", status, err_text);
                        error_code = format!("upstream_{}", status.as_u16());
                        ttfb_ms = http_start.elapsed().as_millis() as u32;
                        yield Ok(Event::default().data(format!("{{\"error\": \"Upstream error {}\"}}", status)));
                    } else {
                        breaker.record_success();
                        let mut byte_stream = response.bytes_stream();
                        let mut buffer = Vec::new();
                        let mut got_first_byte = false;

                        while let Some(chunk_res) = byte_stream.next().await {
                            match chunk_res {
                                Ok(bytes) => {
                                    if !got_first_byte {
                                        ttfb_ms = http_start.elapsed().as_millis() as u32;
                                        got_first_byte = true;
                                    }
                                    buffer.extend_from_slice(&bytes);
                                    
                                    while let Some(pos) = buffer.iter().position(|&b| b == b'\n') {
                                        let line_bytes = buffer.drain(..pos + 1).collect::<Vec<u8>>();
                                        let line = String::from_utf8_lossy(&line_bytes);
                                        let line = line.trim();

                                        if line.starts_with("data: ") {
                                            let json_str = &line[6..];
                                            if let Ok(event) = serde_json::from_str::<AnthropicEvent>(json_str) {
                                                match event {
                                                    AnthropicEvent::MessageStart { message } => {
                                                        // Capture input tokens from message_start
                                                        if let Some(u) = message.usage {
                                                            if let Some(it) = u.input_tokens { prompt_tokens = it; }
                                                        }
                                                    }
                                                    AnthropicEvent::ContentBlockDelta { delta, .. } => {
                                                        if let Some(text) = delta.text {
                                                            let chunk = ChatCompletionChunk {
                                                                id: req_id.clone(),
                                                                object: "chat.completion.chunk".to_string(),
                                                                created: 0,
                                                                model: model_name.clone(),
                                                                choices: vec![ChunkChoice {
                                                                    index: 0,
                                                                    delta: Delta {
                                                                        role: Some("assistant".to_string()),
                                                                        content: Some(text),
                                                                    },
                                                                    finish_reason: None,
                                                                }],
                                                            };
                                                            if let Ok(json) = serde_json::to_string(&chunk) {
                                                                completion_tokens += 1; // chunk count proxy
                                                                yield Ok(Event::default().data(json));
                                                            }
                                                        }
                                                    }
                                                    AnthropicEvent::MessageDelta { usage, .. } => {
                                                        // Final output token count from Anthropic
                                                        if let Some(u) = usage {
                                                            if let Some(ot) = u.output_tokens { completion_tokens = ot; }
                                                        }
                                                    }
                                                    AnthropicEvent::Error { error } => {
                                                        tracing::error!("Anthropic SSE Error: {}", error.message);
                                                    }
                                                    _ => {} // Ignore ping, start, stop
                                                }
                                            }
                                        }
                                    }
                                }
                                Err(e) => {
                                    tracing::error!("Error reading Anthropic stream: {}", e);
                                    break;
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    breaker.record_failure();
                    tracing::error!("Failed to connect to Anthropic API: {}", e);
                    error_code = "connection_failed".to_string();
                    ttfb_ms = http_start.elapsed().as_millis() as u32;
                    yield Ok(Event::default().data("{\"error\": \"Failed to connect to upstream\"}"));
                }
            }
            let latency_ms = started_at.elapsed().as_millis() as u32;
            let cost_usd = crate::providers::pricing::compute_cost_usd(&model_name, prompt_tokens, completion_tokens);
            audit_publisher.publish(crate::reliability::audit::AuditLogRecord {
                request_id: request_id.clone(),
                timestamp: chrono::Utc::now().to_rfc3339(),
                workspace_id,
                virtual_key_id,
                model: model_name.clone(),
                provider_used: "anthropic".to_string(),
                prompt_token_count: prompt_tokens,
                completion_token_count: completion_tokens,
                cost_usd,
                latency_ms,
                ttfb_ms,
                cache_hit: 0,
                error_code,
            });

            yield Ok(Event::default().data("[DONE]"));
        };

        Box::pin(stream)
    }
}
