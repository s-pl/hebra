-- Hebra platform schema (PostgreSQL). Times are unix milliseconds (BIGINT);
-- flags are 0/1 BIGINT so the same queries run on SQLite.

CREATE TABLE accounts (
    id TEXT PRIMARY KEY,
    handle TEXT NOT NULL UNIQUE,
    kind TEXT NOT NULL CHECK (kind IN ('human', 'agent', 'org', 'runner')),
    name TEXT NOT NULL,
    email TEXT,
    avatar_url TEXT,
    owner_id TEXT REFERENCES accounts (id) ON DELETE CASCADE,
    site_admin BIGINT NOT NULL DEFAULT 0,
    created_at BIGINT NOT NULL,
    disabled_at BIGINT
);
CREATE INDEX accounts_by_owner ON accounts (owner_id);

CREATE TABLE identities (
    provider TEXT NOT NULL,
    subject TEXT NOT NULL,
    account_id TEXT NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    created_at BIGINT NOT NULL,
    PRIMARY KEY (provider, subject)
);

CREATE TABLE org_members (
    org_id TEXT NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    account_id TEXT NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    role TEXT NOT NULL CHECK (role IN ('owner', 'admin', 'member', 'billing')),
    created_at BIGINT NOT NULL,
    PRIMARY KEY (org_id, account_id)
);
CREATE INDEX org_members_by_account ON org_members (account_id);

CREATE TABLE teams (
    id TEXT PRIMARY KEY,
    org_id TEXT NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    handle TEXT NOT NULL,
    name TEXT NOT NULL,
    created_at BIGINT NOT NULL,
    UNIQUE (org_id, handle)
);

CREATE TABLE team_members (
    team_id TEXT NOT NULL REFERENCES teams (id) ON DELETE CASCADE,
    account_id TEXT NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    PRIMARY KEY (team_id, account_id)
);

CREATE TABLE sessions (
    id_hash TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    created_at BIGINT NOT NULL,
    expires_at BIGINT NOT NULL,
    last_seen_at BIGINT NOT NULL,
    user_agent TEXT
);
CREATE INDEX sessions_by_account ON sessions (account_id);

CREATE TABLE device_codes (
    code_hash TEXT PRIMARY KEY,
    user_code TEXT NOT NULL UNIQUE,
    client_name TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('pending', 'approved', 'denied', 'used')),
    account_id TEXT REFERENCES accounts (id) ON DELETE CASCADE,
    created_at BIGINT NOT NULL,
    expires_at BIGINT NOT NULL
);

CREATE TABLE invitations (
    code_hash TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    created_at BIGINT NOT NULL,
    expires_at BIGINT NOT NULL,
    used_at BIGINT
);

CREATE TABLE mandates (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    label TEXT NOT NULL,
    created_by TEXT,
    created_at BIGINT NOT NULL,
    expires_at BIGINT,
    revoked_at BIGINT,
    last_used_at BIGINT
);
CREATE INDEX mandates_by_account ON mandates (account_id);

CREATE TABLE revocations (
    revocation_id TEXT PRIMARY KEY,
    revoked_at BIGINT NOT NULL,
    reason TEXT NOT NULL
);

CREATE TABLE repos (
    id TEXT PRIMARY KEY,
    owner_id TEXT NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    full_name TEXT NOT NULL UNIQUE,
    description TEXT NOT NULL DEFAULT '',
    public BIGINT NOT NULL DEFAULT 1,
    open_contributions BIGINT NOT NULL DEFAULT 0,
    forked_from TEXT REFERENCES repos (id) ON DELETE SET NULL,
    -- Ids of the repositories this one reads objects from (forks), nearest first.
    lineage TEXT NOT NULL DEFAULT '[]',
    created_at BIGINT NOT NULL,
    UNIQUE (owner_id, name)
);

CREATE TABLE repo_access (
    repo_id TEXT NOT NULL REFERENCES repos (id) ON DELETE CASCADE,
    principal_id TEXT NOT NULL,
    role TEXT NOT NULL CHECK (role IN ('admin', 'maintain', 'write', 'triage', 'read')),
    PRIMARY KEY (repo_id, principal_id)
);

CREATE TABLE protected (
    repo_id TEXT NOT NULL REFERENCES repos (id) ON DELETE CASCADE,
    pattern TEXT NOT NULL,
    PRIMARY KEY (repo_id, pattern)
);

CREATE TABLE bookmarks (
    repo_id TEXT NOT NULL REFERENCES repos (id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    target TEXT NOT NULL,
    updated_at BIGINT NOT NULL,
    updated_by TEXT NOT NULL,
    PRIMARY KEY (repo_id, name)
);

CREATE TABLE counters (
    scope TEXT PRIMARY KEY,
    value BIGINT NOT NULL
);

CREATE TABLE events (
    id BIGSERIAL PRIMARY KEY,
    repo_id TEXT NOT NULL REFERENCES repos (id) ON DELETE CASCADE,
    at BIGINT NOT NULL,
    actor TEXT NOT NULL,
    kind TEXT NOT NULL,
    detail TEXT NOT NULL
);
CREATE INDEX events_by_repo ON events (repo_id, id);

CREATE TABLE presence (
    repo_id TEXT NOT NULL REFERENCES repos (id) ON DELETE CASCADE,
    client TEXT NOT NULL,
    payload TEXT NOT NULL,
    updated_at BIGINT NOT NULL,
    PRIMARY KEY (repo_id, client)
);
CREATE INDEX presence_by_age ON presence (updated_at);

CREATE TABLE reviews (
    id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL REFERENCES repos (id) ON DELETE CASCADE,
    number BIGINT NOT NULL,
    source TEXT NOT NULL,
    target TEXT NOT NULL,
    title TEXT NOT NULL,
    body TEXT NOT NULL,
    author TEXT NOT NULL,
    author_id TEXT,
    state TEXT NOT NULL CHECK (state IN ('open', 'queued', 'merged', 'closed')),
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL,
    closed_by TEXT,
    merged_commit TEXT,
    UNIQUE (repo_id, number)
);
CREATE INDEX reviews_by_state ON reviews (repo_id, state);

CREATE TABLE review_comments (
    id TEXT PRIMARY KEY,
    review_id TEXT NOT NULL REFERENCES reviews (id) ON DELETE CASCADE,
    author TEXT NOT NULL,
    author_id TEXT,
    path TEXT,
    line BIGINT,
    side TEXT,
    commit_id TEXT,
    body TEXT NOT NULL,
    reply_to TEXT,
    created_at BIGINT NOT NULL,
    resolved_at BIGINT
);
CREATE INDEX review_comments_by_review ON review_comments (review_id, created_at);

CREATE TABLE review_approvals (
    review_id TEXT NOT NULL REFERENCES reviews (id) ON DELETE CASCADE,
    account_id TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('approved', 'changes_requested')),
    commit_id TEXT NOT NULL,
    created_at BIGINT NOT NULL,
    PRIMARY KEY (review_id, account_id)
);

CREATE TABLE tasks (
    id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL REFERENCES repos (id) ON DELETE CASCADE,
    number BIGINT NOT NULL,
    title TEXT NOT NULL,
    body TEXT NOT NULL,
    acceptance TEXT NOT NULL,
    labels TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('open', 'claimed', 'done', 'cancelled')),
    parent_id TEXT REFERENCES tasks (id) ON DELETE SET NULL,
    created_by TEXT NOT NULL,
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL,
    claimed_by TEXT,
    lease_until BIGINT,
    closed_by TEXT,
    UNIQUE (repo_id, number)
);
CREATE INDEX tasks_by_state ON tasks (repo_id, state);

CREATE TABLE task_deps (
    task_id TEXT NOT NULL REFERENCES tasks (id) ON DELETE CASCADE,
    depends_on TEXT NOT NULL REFERENCES tasks (id) ON DELETE CASCADE,
    PRIMARY KEY (task_id, depends_on)
);

CREATE TABLE runners (
    id TEXT PRIMARY KEY,
    owner_id TEXT NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    account_id TEXT NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    labels TEXT NOT NULL,
    created_at BIGINT NOT NULL,
    last_seen_at BIGINT,
    version TEXT,
    UNIQUE (owner_id, name)
);

CREATE TABLE check_runs (
    id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL REFERENCES repos (id) ON DELETE CASCADE,
    commit_id TEXT NOT NULL,
    name TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('command', 'agent')),
    spec TEXT NOT NULL,
    labels TEXT NOT NULL,
    cause TEXT NOT NULL,
    review_id TEXT REFERENCES reviews (id) ON DELETE SET NULL,
    required BIGINT NOT NULL DEFAULT 0,
    state TEXT NOT NULL CHECK (state IN ('waiting', 'queued', 'running', 'success', 'failure', 'error', 'cancelled', 'skipped')),
    runner_id TEXT REFERENCES runners (id) ON DELETE SET NULL,
    attempt BIGINT NOT NULL DEFAULT 0,
    lease_until BIGINT,
    created_at BIGINT NOT NULL,
    started_at BIGINT,
    finished_at BIGINT,
    exit_code BIGINT,
    summary TEXT,
    log_bytes BIGINT NOT NULL DEFAULT 0,
    credits BIGINT NOT NULL DEFAULT 0
);
CREATE INDEX check_runs_by_commit ON check_runs (repo_id, commit_id);
CREATE INDEX check_runs_queue ON check_runs (state, created_at);

CREATE TABLE check_logs (
    run_id TEXT NOT NULL REFERENCES check_runs (id) ON DELETE CASCADE,
    seq BIGINT NOT NULL,
    chunk TEXT NOT NULL,
    PRIMARY KEY (run_id, seq)
);

CREATE TABLE subscriptions (
    owner_id TEXT PRIMARY KEY REFERENCES accounts (id) ON DELETE CASCADE,
    plan TEXT NOT NULL,
    status TEXT NOT NULL,
    stripe_customer TEXT,
    stripe_subscription TEXT,
    seats BIGINT NOT NULL DEFAULT 1,
    period_start BIGINT,
    period_end BIGINT,
    spending_limit_cents BIGINT NOT NULL DEFAULT 0,
    updated_at BIGINT NOT NULL
);

CREATE TABLE usage (
    owner_id TEXT NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    period TEXT NOT NULL,
    metric TEXT NOT NULL,
    quantity BIGINT NOT NULL,
    PRIMARY KEY (owner_id, period, metric)
);

CREATE TABLE usage_events (
    id TEXT PRIMARY KEY,
    owner_id TEXT NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    metric TEXT NOT NULL,
    quantity BIGINT NOT NULL,
    reference TEXT NOT NULL,
    at BIGINT NOT NULL,
    reported_at BIGINT
);
CREATE INDEX usage_events_unreported ON usage_events (reported_at, at);

CREATE TABLE stripe_events (
    id TEXT PRIMARY KEY,
    received_at BIGINT NOT NULL
);

CREATE TABLE log_entries (
    repo_id TEXT NOT NULL REFERENCES repos (id) ON DELETE CASCADE,
    seq BIGINT NOT NULL,
    at BIGINT NOT NULL,
    kind TEXT NOT NULL,
    payload TEXT NOT NULL,
    prev_hash TEXT NOT NULL,
    hash TEXT NOT NULL,
    PRIMARY KEY (repo_id, seq)
);
