use dashmap::DashMap;
use sqlx::PgPool;
use std::time::{Duration, Instant};
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Clone, Debug)]
pub struct KeyMetadata {
    pub workspace_id: Uuid,
    pub expires_at: Option<OffsetDateTime>,
    pub cached_at: Instant,
}

pub struct AuthCache {
    map: DashMap<String, KeyMetadata>,
}

impl AuthCache {
    pub fn new() -> Self {
        Self {
            map: DashMap::new(),
        }
    }

    pub async fn warm_up(&self, pool: &PgPool) -> Result<(), sqlx::Error> {
        let rows = sqlx::query!(
            r#"
            SELECT key_hash, workspace_id, expires_at
            FROM virtual_keys
            WHERE revoked = false
            AND (expires_at IS NULL OR expires_at > now())
            "#
        )
        .fetch_all(pool)
        .await?;

        for row in rows {
            self.map.insert(
                row.key_hash,
                KeyMetadata {
                    workspace_id: row.workspace_id,
                    expires_at: row.expires_at,
                    cached_at: Instant::now(),
                },
            );
        }
        tracing::info!("Auth cache warmed with {} active keys", self.map.len());
        Ok(())
    }

    pub fn get(&self, key_hash: &str) -> Option<KeyMetadata> {
        if let Some(entry) = self.map.get(key_hash) {
            let meta = entry.value();
            if meta.cached_at.elapsed() > Duration::from_secs(30) {
                return None; // Trigger downstream cache-miss TTL
            }
            return Some(meta.clone());
        }
        None
    }

    pub fn insert(&self, key_hash: String, meta: KeyMetadata) {
        self.map.insert(key_hash, meta);
    }
}
