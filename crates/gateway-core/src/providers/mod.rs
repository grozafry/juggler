use axum::response::sse::Event;
use reqwest::Client;
use std::convert::Infallible;
use std::pin::Pin;
use tokio_stream::Stream;
use crate::server::models::ChatCompletionRequest;

pub mod gemini;
pub mod anthropic;
pub mod pricing;

pub trait Provider: Send + Sync {
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
    ) -> Pin<Box<dyn Stream<Item = Result<Event, Infallible>> + Send>>;
}
