-- Pipelines: checks that wait for others, results reused when their inputs
-- did not change, secrets for runs, files runs leave behind, and one-time
-- tokens that let a machine register itself as a runner.

ALTER TABLE check_runs ADD COLUMN needs TEXT;
ALTER TABLE check_runs ADD COLUMN input_key TEXT;
ALTER TABLE check_runs ADD COLUMN reused_from TEXT;
CREATE INDEX check_runs_by_input ON check_runs (repo_id, name, input_key);

-- Secrets of a repository or of an account (all its repositories). Values
-- are sealed with a key derived from the server's signing key.
CREATE TABLE secrets (
    scope TEXT NOT NULL CHECK (scope IN ('repo', 'account')),
    owner_id TEXT NOT NULL,
    name TEXT NOT NULL,
    sealed TEXT NOT NULL,
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL,
    updated_by TEXT,
    PRIMARY KEY (scope, owner_id, name)
);

CREATE TABLE artifacts (
    id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL REFERENCES check_runs (id) ON DELETE CASCADE,
    repo_id TEXT NOT NULL REFERENCES repos (id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    size BIGINT NOT NULL,
    digest TEXT NOT NULL,
    created_at BIGINT NOT NULL,
    UNIQUE (run_id, name)
);
CREATE INDEX artifacts_by_repo ON artifacts (repo_id, created_at);

CREATE TABLE runner_tokens (
    hash TEXT PRIMARY KEY,
    owner_id TEXT NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    labels TEXT NOT NULL,
    ephemeral BIGINT NOT NULL DEFAULT 0,
    uses_left BIGINT NOT NULL,
    expires_at BIGINT NOT NULL,
    created_by TEXT,
    created_at BIGINT NOT NULL
);

ALTER TABLE runners ADD COLUMN ephemeral BIGINT NOT NULL DEFAULT 0;
ALTER TABLE runners ADD COLUMN machine TEXT;
