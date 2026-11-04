-- Environments: where checks deploy to (staging, production…), with rules
-- for who and what may deploy there, their own secrets and runners, and
-- the history of deployments.

CREATE TABLE environments (
    repo_id TEXT NOT NULL REFERENCES repos (id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    -- People who must approve a deployment before it runs.
    required_approvals BIGINT NOT NULL DEFAULT 0,
    -- Bookmark patterns that may deploy here (JSON list; empty: any).
    branches TEXT NOT NULL DEFAULT '[]',
    -- Labels a runner needs to deploy here (a dedicated pool).
    runner_labels TEXT NOT NULL DEFAULT '[]',
    url TEXT,
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL,
    PRIMARY KEY (repo_id, name)
);

CREATE TABLE deployments (
    id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL REFERENCES repos (id) ON DELETE CASCADE,
    environment TEXT NOT NULL,
    run_id TEXT NOT NULL REFERENCES check_runs (id) ON DELETE CASCADE,
    commit_id TEXT NOT NULL,
    bookmark TEXT,
    approved_by TEXT,
    created_at BIGINT NOT NULL
);
CREATE INDEX deployments_by_env ON deployments (repo_id, environment, created_at);

ALTER TABLE check_runs ADD COLUMN environment TEXT;
