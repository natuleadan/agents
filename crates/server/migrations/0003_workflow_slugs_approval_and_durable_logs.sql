-- Phase 12 schema extension: Dynamic webhook slugs, render slugs,
-- role approvals (OWASP ASI-02/ASI-08), and durable execution step logs.

ALTER TABLE workflows
    ADD COLUMN IF NOT EXISTS webhook_slug TEXT UNIQUE,
    ADD COLUMN IF NOT EXISTS render_slug TEXT UNIQUE,
    ADD COLUMN IF NOT EXISTS approved BOOLEAN NOT NULL DEFAULT TRUE,
    ADD COLUMN IF NOT EXISTS approved_by TEXT,
    ADD COLUMN IF NOT EXISTS approved_at BIGINT,
    ADD COLUMN IF NOT EXISTS created_by_agent BOOLEAN NOT NULL DEFAULT FALSE;

CREATE INDEX IF NOT EXISTS idx_workflows_webhook_slug ON workflows(webhook_slug);
CREATE INDEX IF NOT EXISTS idx_workflows_render_slug ON workflows(render_slug);

CREATE TABLE IF NOT EXISTS rendered_slugs (
    slug TEXT PRIMARY KEY,
    workflow_id TEXT NOT NULL REFERENCES workflows(workflow_id) ON DELETE CASCADE,
    content_type TEXT NOT NULL DEFAULT 'text/html; charset=utf-8',
    content TEXT NOT NULL,
    approved BOOLEAN NOT NULL DEFAULT FALSE,
    updated_at BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_rendered_slugs_workflow ON rendered_slugs(workflow_id);

CREATE TABLE IF NOT EXISTS durable_step_logs (
    step_id TEXT PRIMARY KEY,
    task_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    step_index INTEGER NOT NULL,
    step_kind TEXT NOT NULL,
    call_id TEXT,
    tool_name TEXT,
    input_payload JSONB,
    output_payload JSONB,
    is_error BOOLEAN NOT NULL DEFAULT FALSE,
    recorded_at_ms BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_durable_step_logs_task ON durable_step_logs(task_id, step_index ASC);
