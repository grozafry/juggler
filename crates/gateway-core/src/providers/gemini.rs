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
#[serde(rename_all = "camelCase")]
struct GeminiResponse {
    candidates: Option<Vec<GeminiCandidate>>,
    usage_metadata: Option<GeminiUsageMetadata>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct GeminiUsageMetadata {
    prompt_token_count: Option<u32>,
    candidates_token_count: Option<u32>,
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
        breaker: std::sync::Arc<crate::reliability::breaker::CircuitBreaker>,
        request_id: String,
        workspace_id: String,
        virtual_key_id: String,
        audit_publisher: std::sync::Arc<crate::reliability::audit::AuditPublisher>,
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
                let attempt_res = client.post(&url)
                    .header("x-request-id", request_id.clone())
                    .json(&gemini_req)
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
                        tracing::error!("Gemini API returned error {}: {}", status, err_text);
                        error_code = format!("upstream_{}", status.as_u16());
                        ttfb_ms = http_start.elapsed().as_millis() as u32;
                        yield Ok(Event::default().data(format!("{{\"error\": \"Upstream error {}\"}}", status)));
                    } else {
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
                                            if let Ok(gemini_resp) = serde_json::from_str::<GeminiResponse>(json_str) {
                                                // Extract real token counts from usageMetadata (present on final chunk)
                                                if let Some(ref usage) = gemini_resp.usage_metadata {
                                                    if let Some(pt) = usage.prompt_token_count { prompt_tokens = pt; }
                                                    if let Some(ct) = usage.candidates_token_count { completion_tokens = ct; }
                                                }
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
                                                                            completion_tokens += 1; // fallback if usageMetadata not present
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
                    breaker.record_failure();
                    tracing::error!("Failed to connect to Gemini API: {}", e);
                    error_code = "connection_failed".to_string();
                    ttfb_ms = http_start.elapsed().as_millis() as u32;
                    yield Ok(Event::default().data("{\"error\": \"Failed to connect to upstream\"}"));
                }
            }
            let latency_ms = started_at.elapsed().as_millis() as u32;
            // If usageMetadata never arrived (errors), completion_tokens is the chunk count proxy
            let cost_usd = crate::providers::pricing::compute_cost_usd(&model_name, prompt_tokens, completion_tokens);
            audit_publisher.publish(crate::reliability::audit::AuditLogRecord {
                request_id: request_id.clone(),
                timestamp: chrono::Utc::now().to_rfc3339(),
                workspace_id,
                virtual_key_id,
                model: model_name.clone(),
                provider_used: "gemini".to_string(),
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
