use axum::response::sse::Event;
use reqwest::Client;
use std::convert::Infallible;
use std::pin::Pin;
use tokio_stream::Stream;
use crate::server::models::ChatCompletionRequest;

pub mod gemini;
pub mod anthropic;

pub trait Provider: Send + Sync {
    fn stream(
        &self,
        client: Client,
        model: &str,
        request: ChatCompletionRequest,
    ) -> Pin<Box<dyn Stream<Item = Result<Event, Infallible>> + Send>>;
}
