use axum::{
    extract::State,
    response::sse::{Event, Sse},
    response::{Response, IntoResponse},
    Json,
};
use serde_json::json;
use reqwest::Client;
use std::convert::Infallible;
use tokio_stream::Stream;

use crate::providers::{anthropic::AnthropicProvider, gemini::GeminiProvider, Provider};
use crate::server::models::{ChatCompletionRequest, GatewayConfig};
use tracing::Instrument;
use uuid::Uuid;

#[derive(Clone)]
pub struct AppState {
    pub http_client: Client,
    pub config: GatewayConfig,
    pub auth_cache: std::sync::Arc<crate::auth::cache::AuthCache>,
    pub db_pool: sqlx::PgPool,
    pub gemini_cb: std::sync::Arc<crate::reliability::breaker::CircuitBreaker>,
    pub anthropic_cb: std::sync::Arc<crate::reliability::breaker::CircuitBreaker>,
    pub audit_publisher: std::sync::Arc<crate::reliability::audit::AuditPublisher>,
}

pub async fn chat_completions(
    key: crate::auth::middleware::ValidatedKey,
    State(state): State<AppState>,
    Json(payload): Json<ChatCompletionRequest>,
) -> Result<Response, Response> {
    let request_id = Uuid::new_v4().to_string();
    let span = tracing::info_span!("chat_completions", request_id = %request_id, workspace_id = %key.workspace_id);
    
    // Call inner function correctly instrumented
    let res = async {
        chat_completions_inner(key.clone(), state, payload, request_id.clone()).await
    }.instrument(span).await;
    
    match res {
        Ok(mut success_resp) => {
            if let Ok(val) = axum::http::HeaderValue::from_str(&request_id) {
                success_resp.headers_mut().insert("X-Request-ID", val);
            }
            Ok(success_resp)
        }
        Err(mut error_resp) => {
            if let Ok(val) = axum::http::HeaderValue::from_str(&request_id) {
                error_resp.headers_mut().insert("X-Request-ID", val);
            }
            Err(error_resp)
        }
    }
}

async fn chat_completions_inner(
    _key: crate::auth::middleware::ValidatedKey,
    state: AppState,
    payload: ChatCompletionRequest,
    request_id: String,
) -> Result<Response, Response> {
    tracing::info!("Received chat completion request for model: {}", payload.model);
    
    let route = match crate::routing::router::resolve_route_with_state(&payload.model, &state) {
        Ok(r) => r,
        Err(retry_after) => {
            tracing::warn!("All available providers OPEN for alias {}", payload.model);
            return Err((
                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                [(axum::http::header::RETRY_AFTER, retry_after.to_string())],
                Json(json!({
                    "error": "provider_unavailable",
                    "alias": payload.model,
                    "retry_after": retry_after
                })),
            )
                .into_response());
        }
    };

    tracing::info!("Routing to provider: {} (model: {})", route.provider_name, route.internal_model);
    
    let (provider, cb) = match route.provider_name.as_str() {
        "anthropic" => (
            Box::new(AnthropicProvider) as Box<dyn Provider>,
            &state.anthropic_cb,
        ),
        "gemini" | _ => (
            Box::new(GeminiProvider) as Box<dyn Provider>,
            &state.gemini_cb,
        ), // default fallback
    };

    let stream = provider.stream(
        state.http_client.clone(),
        &route.internal_model,
        payload,
        cb.clone(),
        request_id.clone(),
        _key.workspace_id.to_string(),
        "lgw_virtual_token".to_string(), // TODO: add virtual_key_id to ValidatedKey
        state.audit_publisher.clone(),
    );

    Ok(Sse::new(stream).into_response())
}
