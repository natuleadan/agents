-- Initial PostgreSQL schema for agents-server.
-- Stores identity, revocable SHA-256 credentials, sessions, durable fenced tasks,
-- ordered event stream envelopes, and tamper-evident hash-chained audit logs.

CREATE TABLE IF NOT EXISTS users (
    id TEXT PRIMARY KEY,
    email TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    role TEXT NOT NULL DEFAULT 'user',
    scopes JSONB NOT NULL DEFAULT '["read","write"]'::jsonb,
    created_at BIGINT NOT NULL
);

CREATE TABLE IF NOT EXISTS api_keys (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    label TEXT NOT NULL,
    prefix TEXT NOT NULL,
    key_hash TEXT NOT NULL UNIQUE,
    scopes JSONB NOT NULL,
    created_at BIGINT NOT NULL,
    expires_at BIGINT,
    revoked_at BIGINT
);

CREATE INDEX IF NOT EXISTS idx_api_keys_user_id ON api_keys(user_id);

CREATE TABLE IF NOT EXISTS refresh_tokens (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    token_hash TEXT NOT NULL UNIQUE,
    scopes JSONB NOT NULL,
    created_at BIGINT NOT NULL,
    expires_at BIGINT NOT NULL,
    revoked_at BIGINT
);

CREATE INDEX IF NOT EXISTS idx_refresh_tokens_user_id ON refresh_tokens(user_id);

CREATE TABLE IF NOT EXISTS sessions (
    id TEXT PRIMARY KEY,
    owner_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    title TEXT NOT NULL,
    mode TEXT NOT NULL DEFAULT 'act',
    archived_at BIGINT,
    draft_content TEXT,
    draft_updated_at BIGINT,
    version BIGINT NOT NULL DEFAULT 1,
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_sessions_owner_updated ON sessions(owner_id, updated_at DESC, id DESC);
CREATE INDEX IF NOT EXISTS idx_sessions_archived ON sessions(owner_id, archived_at);

CREATE TABLE IF NOT EXISTS tasks (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    owner_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    prompt TEXT NOT NULL,
    state TEXT NOT NULL,
    mode TEXT NOT NULL DEFAULT 'act',
    fencing_token BIGINT NOT NULL DEFAULT 0,
    lease_holder TEXT,
    lease_expires_at BIGINT,
    idempotency_key TEXT,
    version BIGINT NOT NULL DEFAULT 1,
    exit_code INTEGER,
    stop_reason TEXT,
    output_summary TEXT,
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL,
    CONSTRAINT uq_tasks_session_idempotency UNIQUE (session_id, idempotency_key)
);

CREATE INDEX IF NOT EXISTS idx_tasks_session_created ON tasks(session_id, created_at DESC, id DESC);
CREATE INDEX IF NOT EXISTS idx_tasks_state_lease ON tasks(state, lease_expires_at);

CREATE TABLE IF NOT EXISTS stream_events (
    id BIGSERIAL PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    task_id TEXT REFERENCES tasks(id) ON DELETE SET NULL,
    seq BIGINT NOT NULL,
    emitted_at_ms BIGINT NOT NULL,
    codec TEXT NOT NULL,
    payload JSONB NOT NULL,
    CONSTRAINT uq_stream_events_session_seq UNIQUE (session_id, seq)
);

CREATE INDEX IF NOT EXISTS idx_stream_events_session_seq ON stream_events(session_id, seq ASC);

CREATE TABLE IF NOT EXISTS audit_logs (
    id TEXT PRIMARY KEY,
    actor_id TEXT,
    action TEXT NOT NULL,
    resource_type TEXT NOT NULL,
    resource_id TEXT,
    outcome TEXT NOT NULL,
    details JSONB NOT NULL DEFAULT '{}'::jsonb,
    prev_hash TEXT NOT NULL,
    entry_hash TEXT NOT NULL,
    created_at BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_audit_logs_created ON audit_logs(created_at DESC);
CREATE INDEX IF NOT EXISTS idx_audit_logs_actor ON audit_logs(actor_id, created_at DESC);
