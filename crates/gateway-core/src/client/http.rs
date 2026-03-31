use reqwest::Client;
use std::time::Duration;

pub fn build_client() -> Client {
    Client::builder()
        .pool_idle_timeout(Duration::from_secs(90))
        .pool_max_idle_per_host(100)
        .http2_adaptive_window(true)
        .tcp_nodelay(true)
        .build()
        .expect("Failed to build HTTP client pool")
}
