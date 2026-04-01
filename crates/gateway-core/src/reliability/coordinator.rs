use super::breaker::CircuitBreaker;
use tokio_stream::StreamExt;
use serde::Deserialize;
use std::sync::Arc;

#[derive(Deserialize)]
struct CircuitEvent {
    event: String,
    provider: String,
}

pub async fn start_redis_subscriber(
    client: redis::Client,
    gemini_cb: Arc<CircuitBreaker>,
    anthropic_cb: Arc<CircuitBreaker>,
) {
    tokio::spawn(async move {
        loop {
            if let Ok(mut pubsub) = client.get_async_pubsub().await {
                if pubsub.subscribe("gateway:circuit:events").await.is_ok() {
                    let mut stream = pubsub.on_message();
                    while let Some(msg) = stream.next().await {
                        if let Ok(payload) = msg.get_payload::<String>() {
                            if let Ok(event) = serde_json::from_str::<CircuitEvent>(&payload) {
                                let target_cb = match event.provider.as_str() {
                                    "gemini" => &gemini_cb,
                                    "anthropic" => &anthropic_cb,
                                    _ => continue,
                                };
                                match event.event.as_str() {
                                    "tripped" => target_cb.set_open(),
                                    "recovered" => target_cb.set_closed(),
                                    _ => {}
                                }
                            }
                        }
                    }
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        }
    });
}
