-- Drop and recreate to pick up all fixes cleanly
DROP TABLE IF EXISTS kafka_to_audit_logs;
DROP TABLE IF EXISTS audit_kafka_queue;
DROP TABLE IF EXISTS audit_logs;

CREATE TABLE IF NOT EXISTS audit_logs (
    request_id String,
    timestamp   DateTime64(3, 'UTC') DEFAULT now64(),
    workspace_id String,
    virtual_key_id String,
    model String,
    provider_used String,
    prompt_token_count UInt32,
    completion_token_count UInt32,
    cost_usd Float64,
    latency_ms UInt32,
    cache_hit UInt8,
    error_code String
) ENGINE = MergeTree()
ORDER BY (workspace_id, timestamp);

CREATE TABLE IF NOT EXISTS audit_kafka_queue (
    request_id String,
    timestamp String,
    workspace_id String,
    virtual_key_id String,
    model String,
    provider_used String,
    prompt_token_count UInt32,
    completion_token_count UInt32,
    cost_usd Float64,
    latency_ms UInt32,
    cache_hit UInt8,
    error_code String
) ENGINE = Kafka
SETTINGS kafka_broker_list = 'redpanda:9092',
         kafka_topic_list = 'audit_logs',
         kafka_group_name = 'clickhouse_audit_consumer',
         kafka_format = 'JSONEachRow';

CREATE MATERIALIZED VIEW IF NOT EXISTS kafka_to_audit_logs TO audit_logs AS
SELECT
    request_id,
    coalesce(parseDateTime64BestEffortOrNull(timestamp, 3, 'UTC'), now64()) AS timestamp,
    workspace_id,
    virtual_key_id,
    model,
    provider_used,
    prompt_token_count,
    completion_token_count,
    cost_usd,
    latency_ms,
    cache_hit,
    error_code
FROM audit_kafka_queue;
