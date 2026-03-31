use axum::{
    extract::State,
    response::sse::{Event, Sse},
    Json,
};
use reqwest::Client;
use std::convert::Infallible;
use tokio_stream::Stream;

use crate::providers::{anthropic::AnthropicProvider, gemini::GeminiProvider, Provider};
use crate::routing::router::{resolve_route, GatewayConfig};
use crate::server::models::ChatCompletionRequest;

#[derive(Clone)]
pub struct AppState {
    pub http_client: Client,
    pub config: GatewayConfig,
}

pub async fn chat_completions(
    State(state): State<AppState>,
    Json(payload): Json<ChatCompletionRequest>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    tracing::info!("Received chat completion request for model: {}", payload.model);
    
    let route = resolve_route(&payload.model, &state.config);
    tracing::info!("Routing to provider: {} (model: {})", route.provider_name, route.internal_model);
    
    let provider: Box<dyn Provider> = match route.provider_name.as_str() {
        "anthropic" => Box::new(AnthropicProvider),
        "gemini" => Box::new(GeminiProvider),
        _ => Box::new(GeminiProvider), // default fallback
    };

    let stream = provider.stream(state.http_client, &route.internal_model, payload);
    
    Sse::new(stream)
}
