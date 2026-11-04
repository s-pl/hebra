-- Fleets: runner machines Heddle starts and stops for an owner at a
-- provider (a cloud API, or Docker on the server's host).

CREATE TABLE fleets (
    owner_id TEXT PRIMARY KEY REFERENCES accounts (id) ON DELETE CASCADE,
    enabled BIGINT NOT NULL DEFAULT 0,
    provider TEXT NOT NULL,
    -- Provider settings (region, image, network…) and Heddle's limits, JSON.
    config TEXT NOT NULL,
    -- The provider's API token, sealed.
    credentials TEXT,
    updated_at BIGINT NOT NULL,
    updated_by TEXT
);

CREATE TABLE machines (
    id TEXT PRIMARY KEY,
    owner_id TEXT NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    provider TEXT NOT NULL,
    provider_id TEXT,
    name TEXT NOT NULL,
    kind TEXT NOT NULL,
    -- provisioning, booting, ready, deleting, gone, failed
    state TEXT NOT NULL,
    -- Price per hour and cost so far, in millionths of the provider's currency.
    price_micros BIGINT NOT NULL,
    billing TEXT NOT NULL,
    cost_micros BIGINT NOT NULL DEFAULT 0,
    address TEXT,
    reason TEXT,
    created_at BIGINT NOT NULL,
    ready_at BIGINT,
    ended_at BIGINT,
    UNIQUE (owner_id, name)
);
CREATE INDEX machines_live ON machines (owner_id, state);

CREATE TABLE fleet_events (
    id TEXT PRIMARY KEY,
    owner_id TEXT NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    at BIGINT NOT NULL,
    kind TEXT NOT NULL,
    machine TEXT,
    detail TEXT NOT NULL
);
CREATE INDEX fleet_events_recent ON fleet_events (owner_id, at);
