use axum::{routing::post, Router};
use std::net::SocketAddr;
use tracing::info;

pub mod client;
pub mod providers;
pub mod routing;
pub mod server;
pub mod auth;
pub mod reliability;

// Removed unused handler imports since we use them by full path below
#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();
    let fmt_layer = tracing_subscriber::fmt::layer()
        .json()
        .with_current_span(true);
        
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));

    use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

    if let Ok(endpoint) = std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT") {
        use opentelemetry_sdk::{trace::{Sampler}, Resource};
        use opentelemetry::KeyValue;
        use opentelemetry_otlp::WithExportConfig;

        let tracer = opentelemetry_otlp::new_pipeline()
            .tracing()
            .with_exporter(opentelemetry_otlp::new_exporter().tonic().with_endpoint(endpoint))
            .with_trace_config(
                opentelemetry_sdk::trace::config()
                    .with_sampler(Sampler::AlwaysOn)
                    .with_resource(Resource::new(vec![KeyValue::new("service.name", "gateway-core")])),
            )
            .install_batch(opentelemetry_sdk::runtime::Tokio)
            .expect("Failed to init OTLP");

        let telemetry = tracing_opentelemetry::layer().with_tracer(tracer);
        tracing_subscriber::registry()
            .with(env_filter)
            .with(fmt_layer)
            .with(telemetry)
            .init();
    } else {
        tracing_subscriber::registry()
            .with(env_filter)
            .with(fmt_layer)
            .init();
    }

    // Load config from file
    let config_file = std::fs::read_to_string("config.yaml").unwrap_or_else(|_| String::new());
    let config: server::models::GatewayConfig = serde_yaml::from_str(&config_file)
        .unwrap_or_else(|_| {
            tracing::warn!("Failed to parse config.yaml, using default config");
            server::models::GatewayConfig {
                routing: server::models::RoutingConfig {
                    aliases: std::collections::HashMap::new(),
                },
            }
        });

    let db_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let db_pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .connect(&db_url)
        .await
        .expect("Failed to connect to Postgres");

    let auth_cache = std::sync::Arc::new(auth::cache::AuthCache::new());
    if let Err(e) = auth_cache.warm_up(&db_pool).await {
        tracing::error!("Failed to warm up auth cache: {}", e);
    }

    let audit_publisher = std::sync::Arc::new(reliability::audit::AuditPublisher::new().await);

    let redis_client = redis::Client::open("redis://127.0.0.1:6380").ok();

    let gemini_cb = std::sync::Arc::new(reliability::breaker::CircuitBreaker::new(
        "gemini".to_string(),
        60,
        redis_client.clone(),
    ));

    let anthropic_cb = std::sync::Arc::new(reliability::breaker::CircuitBreaker::new(
        "anthropic".to_string(),
        90,
        redis_client.clone(),
    ));

    if let Some(r_client) = redis_client {
        reliability::coordinator::start_redis_subscriber(
            r_client,
            gemini_cb.clone(),
            anthropic_cb.clone(),
        ).await;
    }

    let http_client = client::http::build_client();

    let state = server::handlers::AppState { 
        http_client, 
        config,
        auth_cache,
        db_pool,
        gemini_cb,
        anthropic_cb,
        audit_publisher,
    };

    let app = Router::new()
        .route("/v1/chat/completions", post(server::handlers::chat_completions))
        .with_state(state);

    let auth = "0.0.0.0:8080".parse::<SocketAddr>().unwrap();
    let listener = tokio::net::TcpListener::bind(auth).await.unwrap();

    info!("Proxy gateway listening on {}", auth);
    axum::serve(listener, app).await.unwrap();
}
