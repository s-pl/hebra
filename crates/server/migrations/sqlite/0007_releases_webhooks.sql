-- Releases: notes and files attached to a tag. Webhooks: events sent to
-- other services, signed, with a record of each delivery.

CREATE TABLE releases (
    id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL REFERENCES repos (id) ON DELETE CASCADE,
    tag TEXT NOT NULL,
    commit_id TEXT NOT NULL,
    title TEXT NOT NULL,
    notes TEXT NOT NULL DEFAULT '',
    draft BIGINT NOT NULL DEFAULT 0,
    prerelease BIGINT NOT NULL DEFAULT 0,
    created_by TEXT NOT NULL,
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL,
    UNIQUE (repo_id, tag)
);

CREATE TABLE release_assets (
    id TEXT PRIMARY KEY,
    release_id TEXT NOT NULL REFERENCES releases (id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    size BIGINT NOT NULL,
    digest TEXT NOT NULL,
    content_type TEXT NOT NULL,
    downloads BIGINT NOT NULL DEFAULT 0,
    created_at BIGINT NOT NULL,
    UNIQUE (release_id, name)
);

CREATE TABLE webhooks (
    id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL REFERENCES repos (id) ON DELETE CASCADE,
    url TEXT NOT NULL,
    secret TEXT NOT NULL,
    events TEXT NOT NULL,
    active BIGINT NOT NULL DEFAULT 1,
    created_by TEXT NOT NULL,
    created_at BIGINT NOT NULL
);
CREATE INDEX webhooks_by_repo ON webhooks (repo_id);

CREATE TABLE webhook_deliveries (
    id TEXT PRIMARY KEY,
    hook_id TEXT NOT NULL REFERENCES webhooks (id) ON DELETE CASCADE,
    event TEXT NOT NULL,
    status BIGINT,
    error TEXT,
    duration_ms BIGINT NOT NULL,
    attempt BIGINT NOT NULL,
    payload TEXT NOT NULL,
    at BIGINT NOT NULL
);
CREATE INDEX webhook_deliveries_recent ON webhook_deliveries (hook_id, at);
