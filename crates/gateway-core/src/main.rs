use axum::{routing::post, Router};
use std::net::SocketAddr;
use tracing::info;

pub mod client;
pub mod providers;
pub mod routing;
pub mod server;

// Removed unused handler imports since we use them by full path below
#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt::init();

    // Load config from file
    let config_file = std::fs::read_to_string("config.yaml").unwrap_or_else(|_| String::new());
    let config: routing::router::GatewayConfig = serde_yaml::from_str(&config_file)
        .unwrap_or_else(|e| {
            tracing::warn!("Failed to parse config.yaml: {}", e);
            routing::router::GatewayConfig {
                routing: routing::router::RoutingConfig {
                    aliases: std::collections::HashMap::new(),
                },
            }
        });

    let http_client = client::http::build_client();

    let state = server::handlers::AppState { http_client, config };

    let app = Router::new()
        .route("/v1/chat/completions", post(server::handlers::chat_completions))
        .with_state(state);

    let auth = "0.0.0.0:8080".parse::<SocketAddr>().unwrap();
    let listener = tokio::net::TcpListener::bind(auth).await.unwrap();

    info!("Proxy gateway listening on {}", auth);
    axum::serve(listener, app).await.unwrap();
}
