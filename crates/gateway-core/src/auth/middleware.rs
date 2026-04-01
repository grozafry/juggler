use super::cache::{AuthCache, KeyMetadata};
use super::hash_key;
use crate::server::handlers::AppState;
use axum::{
    extract::FromRequestParts,
    http::{request::Parts, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use sqlx::PgPool;
use std::sync::Arc;
use std::time::Instant;
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Clone, Debug)]
pub struct ValidatedKey {
    pub workspace_id: Uuid,
}

#[axum::async_trait]
impl FromRequestParts<AppState> for ValidatedKey {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let auth_header = parts
            .headers
            .get("Authorization")
            .and_then(|h| h.to_str().ok())
            .unwrap_or_default();

        if !auth_header.starts_with("Bearer ") {
            return Err((
                StatusCode::UNAUTHORIZED,
                Json(json!({"error": "missing_authorization_header"})),
            )
                .into_response());
        }

        let token = &auth_header[7..];
        let key_hash = hash_key(token);

        let cache = &state.auth_cache;
        let pool = &state.db_pool;

        // 3. Check dashmap by hash
        if let Some(meta) = cache.get(&key_hash) {
            return Ok(ValidatedKey {
                workspace_id: meta.workspace_id,
            });
        }

        // 4. Postgres Miss Logic
        let row_opt = sqlx::query!(
            r#"
            SELECT workspace_id, expires_at, revoked
            FROM virtual_keys
            WHERE key_hash = $1
            "#,
            key_hash
        )
        .fetch_optional(pool)
        .await
        .map_err(|e| {
            tracing::error!("Auth DB Error: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": "database_error"})),
            )
                .into_response()
        })?;

        if let Some(row) = row_opt {
            if row.revoked.unwrap_or(false) {
                return Err((
                    StatusCode::UNAUTHORIZED,
                    Json(json!({"error": "invalid_api_key"})), // Don't distinguish between revoked and non-existent
                )
                    .into_response());
            }

            if let Some(expires_at) = row.expires_at {
                if expires_at < OffsetDateTime::now_utc() {
                    return Err((
                        StatusCode::UNAUTHORIZED,
                        Json(json!({"error": "api_key_expired"})),
                    )
                        .into_response());
                }
            }

            let new_meta = KeyMetadata {
                workspace_id: row.workspace_id,
                expires_at: row.expires_at,
                cached_at: Instant::now(),
            };
            
            cache.insert(key_hash, new_meta.clone());
            
            Ok(ValidatedKey {
                workspace_id: new_meta.workspace_id,
            })
        } else {
            Err((
                StatusCode::UNAUTHORIZED,
                Json(json!({"error": "invalid_api_key"})),
            )
                .into_response())
        }
    }
}
