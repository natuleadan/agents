-- Phase 4 schema extension: Swarm coordination, DAG workflows, and Autocron
-- schedules with budget caps and crash-resume `boot_id` recovery checkpoints.

CREATE TABLE IF NOT EXISTS swarm_handoffs (
    handoff_id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    owner_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    parent_agent_id TEXT NOT NULL,
    child_agent_id TEXT NOT NULL,
    worktree_branch TEXT,
    delegated_mask JSONB NOT NULL,
    goal TEXT NOT NULL,
    created_at BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_swarm_handoffs_session ON swarm_handoffs(session_id, created_at DESC);

CREATE TABLE IF NOT EXISTS swarm_messages (
    message_id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    from_agent_id TEXT NOT NULL,
    topic TEXT NOT NULL,
    payload JSONB NOT NULL,
    permission_mask JSONB NOT NULL,
    signature TEXT NOT NULL,
    mediated_by_user TEXT,
    emitted_at_ms BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_swarm_messages_session ON swarm_messages(session_id, emitted_at_ms ASC);

CREATE TABLE IF NOT EXISTS workflows (
    workflow_id TEXT PRIMARY KEY,
    owner_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    nodes JSONB NOT NULL,
    edges JSONB NOT NULL,
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_workflows_owner ON workflows(owner_id, updated_at DESC);

CREATE TABLE IF NOT EXISTS autocron_schedules (
    schedule_id TEXT PRIMARY KEY,
    owner_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    workflow_id TEXT NOT NULL REFERENCES workflows(workflow_id) ON DELETE CASCADE,
    trigger_kind TEXT NOT NULL,
    cron_expr TEXT,
    interval_seconds BIGINT,
    timezone TEXT NOT NULL DEFAULT 'UTC',
    max_budget_usd_cents BIGINT,
    spent_budget_usd_cents BIGINT NOT NULL DEFAULT 0,
    ambient_mode BOOLEAN NOT NULL DEFAULT FALSE,
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    next_run_at BIGINT,
    last_run_at BIGINT,
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_autocron_owner ON autocron_schedules(owner_id, enabled);
CREATE INDEX IF NOT EXISTS idx_autocron_next_run ON autocron_schedules(enabled, next_run_at);

CREATE TABLE IF NOT EXISTS workflow_runs (
    run_id TEXT PRIMARY KEY,
    workflow_id TEXT NOT NULL REFERENCES workflows(workflow_id) ON DELETE CASCADE,
    schedule_id TEXT REFERENCES autocron_schedules(schedule_id) ON DELETE SET NULL,
    owner_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    boot_id TEXT NOT NULL,
    status TEXT NOT NULL,
    current_node_id TEXT,
    checkpoint_state JSONB NOT NULL DEFAULT '{}'::jsonb,
    resumed_count INTEGER NOT NULL DEFAULT 0,
    started_at BIGINT NOT NULL,
    completed_at BIGINT
);

CREATE INDEX IF NOT EXISTS idx_workflow_runs_boot_status ON workflow_runs(status, boot_id);
CREATE INDEX IF NOT EXISTS idx_workflow_runs_workflow ON workflow_runs(workflow_id, started_at DESC);
