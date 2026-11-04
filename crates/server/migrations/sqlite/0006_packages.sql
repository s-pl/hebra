-- Packages: npm packages and container images, published to this server.
-- Their files live in the large-file store, by SHA-256.

CREATE TABLE packages (
    id TEXT PRIMARY KEY,
    owner_id TEXT NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN ('npm', 'container')),
    -- npm: `@owner/name`; container: `owner/name` (more segments allowed).
    name TEXT NOT NULL,
    public BIGINT NOT NULL DEFAULT 0,
    description TEXT NOT NULL DEFAULT '',
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL,
    UNIQUE (kind, name)
);
CREATE INDEX packages_by_owner ON packages (owner_id, updated_at);

-- npm: one row per version (the tarball). Container: one row per manifest,
-- named by its digest.
CREATE TABLE package_versions (
    package_id TEXT NOT NULL REFERENCES packages (id) ON DELETE CASCADE,
    version TEXT NOT NULL,
    digest TEXT NOT NULL,
    size BIGINT NOT NULL,
    meta TEXT NOT NULL,
    published_by TEXT,
    created_at BIGINT NOT NULL,
    PRIMARY KEY (package_id, version)
);

-- npm dist-tags and container tags.
CREATE TABLE package_tags (
    package_id TEXT NOT NULL REFERENCES packages (id) ON DELETE CASCADE,
    tag TEXT NOT NULL,
    version TEXT NOT NULL,
    updated_at BIGINT NOT NULL,
    PRIMARY KEY (package_id, tag)
);

-- Container layers and configs uploaded to (or mounted into) an image.
CREATE TABLE package_blobs (
    package_id TEXT NOT NULL REFERENCES packages (id) ON DELETE CASCADE,
    digest TEXT NOT NULL,
    size BIGINT NOT NULL,
    PRIMARY KEY (package_id, digest)
);

-- Container uploads in progress.
CREATE TABLE package_uploads (
    id TEXT PRIMARY KEY,
    package_id TEXT NOT NULL REFERENCES packages (id) ON DELETE CASCADE,
    size BIGINT NOT NULL DEFAULT 0,
    created_at BIGINT NOT NULL
);
