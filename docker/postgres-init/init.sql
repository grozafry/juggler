-- =============================================================
-- Juggler LLM Gateway — Postgres Schema
-- Auto-applied on first docker-compose up via initdb.d
-- =============================================================

CREATE EXTENSION IF NOT EXISTS "pgcrypto";

-- Workspaces — logical grouping of virtual keys
CREATE TABLE IF NOT EXISTS workspaces (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name        TEXT NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Virtual keys — issued to teams/apps in place of real provider keys
CREATE TABLE IF NOT EXISTS virtual_keys (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    key_hash    TEXT NOT NULL UNIQUE,   -- SHA-256 of the raw lgw_sk_* token
    workspace_id UUID NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    name        TEXT NOT NULL DEFAULT '',
    revoked     BOOLEAN NOT NULL DEFAULT false,
    expires_at  TIMESTAMPTZ,            -- NULL = never expires
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_virtual_keys_hash ON virtual_keys(key_hash);
CREATE INDEX IF NOT EXISTS idx_virtual_keys_workspace ON virtual_keys(workspace_id);

-- ── Seed: default workspace + a starter virtual key ────────────────────────
-- Key token: lgw_sk_default1234567890abcdef  (SHA-256 hash below)
-- Only inserted if no workspace exists yet.

INSERT INTO workspaces (id, name)
VALUES ('11111111-2222-3333-4444-555555555555', 'Default Workspace')
ON CONFLICT DO NOTHING;

-- SHA-256 of "lgw_sk_default1234567890abcdef"
INSERT INTO virtual_keys (key_hash, workspace_id, name)
VALUES (
    encode(sha256('lgw_sk_default1234567890abcdef'::bytea), 'hex'),
    '11111111-2222-3333-4444-555555555555',
    'Starter Key (replace me)'
)
ON CONFLICT DO NOTHING;
