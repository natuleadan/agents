-- Installation-wide identity (singleton), indefinite agent interaction ledger, and A2A tasks.

CREATE TABLE IF NOT EXISTS installation_identity (
    singleton_lock BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (singleton_lock),
    organization_name TEXT NOT NULL,
    installation_id TEXT NOT NULL,
    identity_code TEXT,
    registrar_url TEXT,
    strict_inbound_webhooks BOOLEAN NOT NULL DEFAULT FALSE,
    strict_outbound_egress BOOLEAN NOT NULL DEFAULT TRUE,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS agent_interaction_ledger (
    id UUID PRIMARY KEY,
    direction TEXT NOT NULL CHECK (direction IN ('inbound', 'outbound')),
    protocol TEXT NOT NULL,
    remote_org TEXT,
    remote_installation_id TEXT,
    identity_code_present BOOLEAN NOT NULL DEFAULT FALSE,
    verification_status TEXT NOT NULL CHECK (verification_status IN ('verified', 'unverified', 'missing')),
    remote_ip TEXT,
    target_host TEXT,
    endpoint_path TEXT NOT NULL,
    method TEXT NOT NULL,
    status_code INTEGER,
    trace_id TEXT,
    task_id TEXT,
    detail_json JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_agent_interaction_ledger_created
    ON agent_interaction_ledger (created_at DESC);

CREATE INDEX IF NOT EXISTS idx_agent_interaction_ledger_dir_proto
    ON agent_interaction_ledger (direction, protocol, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_agent_interaction_ledger_remote
    ON agent_interaction_ledger (remote_org, remote_installation_id);

CREATE TABLE IF NOT EXISTS a2a_tasks (
    id TEXT PRIMARY KEY,
    context_id TEXT NOT NULL,
    status_state TEXT NOT NULL,
    task_json JSONB NOT NULL,
    caller_org TEXT,
    caller_installation_id TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_a2a_tasks_context
    ON a2a_tasks (context_id, updated_at DESC);
