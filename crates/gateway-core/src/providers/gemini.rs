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

pub struct GeminiProvider;

#[derive(Serialize)]
struct GeminiPart {
    text: String,
}

#[derive(Serialize)]
struct GeminiContent {
    role: String,
    parts: Vec<GeminiPart>,
}

#[derive(Serialize)]
struct GeminiRequest {
    contents: Vec<GeminiContent>,
}

#[derive(Deserialize, Debug)]
struct GeminiResponse {
    candidates: Option<Vec<GeminiCandidate>>,
}

#[derive(Deserialize, Debug)]
struct GeminiCandidate {
    content: Option<GeminiResponseContent>,
}

#[derive(Deserialize, Debug)]
struct GeminiResponseContent {
    parts: Option<Vec<GeminiResponsePart>>,
}

#[derive(Deserialize, Debug)]
struct GeminiResponsePart {
    text: Option<String>,
}

impl Provider for GeminiProvider {
    fn stream(
        &self,
        client: Client,
        model: &str,
        request: ChatCompletionRequest,
    ) -> Pin<Box<dyn Stream<Item = Result<Event, Infallible>> + Send>> {
        let api_key = env::var("GEMINI_API_KEY").unwrap_or_default();
        if api_key.is_empty() {
            tracing::warn!("GEMINI_API_KEY is not set in the environment");
        }

        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:streamGenerateContent?alt=sse&key={}",
            model, api_key
        );

        let contents: Vec<GeminiContent> = request
            .messages
            .into_iter()
            .map(|m| {
                let role = if m.role == "user" { "user" } else { "model" };
                GeminiContent {
                    role: role.to_string(),
                    parts: vec![GeminiPart { text: m.content }],
                }
            })
            .collect();

        let gemini_req = GeminiRequest { contents };
        let req_id = Uuid::new_v4().to_string();
        let model_name = model.to_string();

        let stream = async_stream::stream! {
            tracing::info!("Connecting to Gemini API: {}", url.split('?').next().unwrap_or(""));
            let res = client.post(&url).json(&gemini_req).send().await;

            match res {
                Ok(response) => {
                    let status = response.status();
                    if !status.is_success() {
                        let err_text = response.text().await.unwrap_or_default();
                        tracing::error!("Gemini API returned error {}: {}", status, err_text);
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
                                            if let Ok(gemini_resp) = serde_json::from_str::<GeminiResponse>(json_str) {
                                                if let Some(candidates) = gemini_resp.candidates {
                                                    if let Some(candidate) = candidates.first() {
                                                        if let Some(content) = &candidate.content {
                                                            if let Some(parts) = &content.parts {
                                                                if let Some(part) = parts.first() {
                                                                    if let Some(text) = &part.text {
                                                                        let chunk = ChatCompletionChunk {
                                                                            id: req_id.clone(),
                                                                            object: "chat.completion.chunk".to_string(),
                                                                            created: 0,
                                                                            model: model_name.clone(),
                                                                            choices: vec![ChunkChoice {
                                                                                index: 0,
                                                                                delta: Delta {
                                                                                    role: Some("assistant".to_string()),
                                                                                    content: Some(text.clone()),
                                                                                },
                                                                                finish_reason: None,
                                                                            }],
                                                                        };
                                                                        if let Ok(json) = serde_json::to_string(&chunk) {
                                                                            yield Ok(Event::default().data(json));
                                                                        }
                                                                    }
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                                Err(e) => {
                                    tracing::error!("Error reading Gemini stream: {}", e);
                                    break;
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    tracing::error!("Failed to connect to Gemini API: {}", e);
                    yield Ok(Event::default().data("{\"error\": \"Failed to connect to upstream\"}"));
                }
            }
            yield Ok(Event::default().data("[DONE]"));
        };

        Box::pin(stream)
    }
}
