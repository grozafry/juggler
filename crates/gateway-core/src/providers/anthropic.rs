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
    MessageDelta { delta: AnthropicMessageDelta },
    #[serde(rename = "message_stop")]
    MessageStop {},
    #[serde(rename = "error")]
    Error { error: AnthropicError },
}

#[allow(dead_code)]
#[derive(Deserialize, Debug)]
struct AnthropicMessageObj {
    id: String,
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
            let res = client.post("https://api.anthropic.com/v1/messages")
                .header("x-api-key", api_key)
                .header("anthropic-version", "2023-06-01")
                .json(&anthropic_req)
                .send()
                .await;

            match res {
                Ok(response) => {
                    let status = response.status();
                    if !status.is_success() {
                        let err_text = response.text().await.unwrap_or_default();
                        tracing::error!("Anthropic API returned error {}: {}", status, err_text);
                        yield Ok(Event::default().data(format!("{{\"error\": \"Upstream error {}\"}}", status)));
                    } else {
                        let mut byte_stream = response.bytes_stream();
                        let mut buffer = Vec::new();

                        while let Some(chunk_res) = byte_stream.next().await {
                            match chunk_res {
                                Ok(bytes) => {
                                    buffer.extend_from_slice(&bytes);
                                    
                                    while let Some(pos) = buffer.iter().position(|&b| b == b'\n') {
                                        let line_bytes = buffer.drain(..pos + 1).collect::<Vec<u8>>();
                                        let line = String::from_utf8_lossy(&line_bytes);
                                        let line = line.trim();

                                        if line.starts_with("data: ") {
                                            let json_str = &line[6..];
                                            if let Ok(event) = serde_json::from_str::<AnthropicEvent>(json_str) {
                                                match event {
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
                                                                yield Ok(Event::default().data(json));
                                                            }
                                                        }
                                                    }
                                                    AnthropicEvent::MessageDelta { delta: _ } => {
                                                        // We can handle finish_reason mapping here if needed.
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
                    tracing::error!("Failed to connect to Anthropic API: {}", e);
                    yield Ok(Event::default().data("{\"error\": \"Failed to connect to upstream\"}"));
                }
            }
            yield Ok(Event::default().data("[DONE]"));
        };

        Box::pin(stream)
    }
}
