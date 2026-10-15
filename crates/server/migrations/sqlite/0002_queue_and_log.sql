-- Singleton jobs (merge queue, usage reporting, storage metering) run on one
-- replica at a time: whoever holds a lease on the job's name.
CREATE TABLE leases (
    name TEXT PRIMARY KEY,
    holder TEXT NOT NULL,
    until_ms BIGINT NOT NULL
);

-- Merge queue: one batch at a time per target bookmark. A batch merges
-- several approved reviews on top of the target and waits for the required
-- checks on the result.
CREATE TABLE merge_batches (
    id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL REFERENCES repos (id) ON DELETE CASCADE,
    target TEXT NOT NULL,
    base_commit TEXT NOT NULL,
    batch_commit TEXT NOT NULL,
    reviews TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('testing', 'merged', 'failed', 'stale')),
    created_at BIGINT NOT NULL,
    finished_at BIGINT,
    detail TEXT
);
CREATE INDEX merge_batches_active ON merge_batches (repo_id, target, state);

-- How many reviews the next batch of a target may take (halved on failure,
-- doubled on success: bisection without bookkeeping).
CREATE TABLE merge_queue_limits (
    repo_id TEXT NOT NULL REFERENCES repos (id) ON DELETE CASCADE,
    target TEXT NOT NULL,
    batch_limit BIGINT NOT NULL,
    PRIMARY KEY (repo_id, target)
);
