use serde::Serialize;
use rskafka::client::ClientBuilder;
use rskafka::record::Record;
use rskafka::client::partition::{UnknownTopicHandling, Compression};
use tokio::sync::mpsc;
use tracing::error;

#[derive(Serialize, Debug, Clone)]
pub struct AuditLogRecord {
    pub request_id: String,
    pub timestamp: String,  // RFC3339 format
    pub workspace_id: String,
    pub virtual_key_id: String,
    pub model: String,
    pub provider_used: String,
    pub prompt_token_count: u32,
    pub completion_token_count: u32,
    pub cost_usd: f64,
    pub latency_ms: u32,   // total end-to-end (from stream start to [DONE])
    pub ttfb_ms: u32,      // time to first byte from provider (API processing time)
    pub cache_hit: u8,
    pub error_code: String,
}

pub struct AuditPublisher {
    sender: mpsc::Sender<AuditLogRecord>,
}

impl AuditPublisher {
    pub async fn new() -> Self {
        let (tx, mut rx) = mpsc::channel::<AuditLogRecord>(10000);
        let brokers = vec!["localhost:19092".to_string()]; // Match Redpanda mapped port
        
        tokio::spawn(async move {
            let mut client = None;
            for _ in 0..5 {
                match ClientBuilder::new(brokers.clone()).build().await {
                    Ok(c) => {
                        client = Some(c);
                        tracing::info!("Kafka connected for Audit Publisher!");
                        break;
                    }
                    Err(e) => {
                        error!("Failed to connect to Kafka {}, retrying...", e);
                        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    }
                }
            }

            let client = match client {
                Some(c) => c,
                None => {
                    error!("Audit publisher failed to initialize: No Kafka connection.");
                    return;
                }
            };
            
            let partition_client = match client.partition_client("audit_logs", 0, UnknownTopicHandling::Retry).await {
                Ok(pc) => pc,
                Err(e) => {
                    error!("Failed to get partition client for audit_logs: {}", e);
                    return;
                }
            };

            while let Some(record) = rx.recv().await {
                if let Ok(json_bytes) = serde_json::to_vec(&record) {
                    let kafka_record = Record {
                        key: Some(record.workspace_id.into_bytes()),
                        value: Some(json_bytes),
                        headers: std::collections::BTreeMap::new(),
                        // Using current system time mapped via chrono or time isn't strictly needed for the `Record` structure on some rskafka versions.
                        // We will map it to chrono::Utc::now()
                        timestamp: chrono::Utc::now(),
                    };
                    
                    if let Err(e) = partition_client.produce(vec![kafka_record], Compression::NoCompression).await {
                        error!("Kafka Produce failed: {}", e);
                    }
                }
            }
        });
        
        Self { sender: tx }
    }
    
    pub fn publish(&self, record: AuditLogRecord) {
        if let Err(e) = self.sender.try_send(record) {
            error!("Failed to queue audit record: {}", e);
        }
    }
}
