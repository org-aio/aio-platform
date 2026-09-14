CREATE TABLE IF NOT EXISTS component_sources (
    id UUID PRIMARY KEY,
    git TEXT NOT NULL UNIQUE,
    parent_git TEXT,
    CHECK (git IS DISTINCT FROM parent_git)
);
CREATE TABLE IF NOT EXISTS component_versions (
    digest TEXT PRIMARY KEY,
    source_id UUID NOT NULL REFERENCES component_sources(id),
    archive BYTEA NOT NULL,
    version TEXT NOT NULL,
    source_commit TEXT NOT NULL,
    description JSONB NOT NULL,
    metadata JSONB NOT NULL,
    readme TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE TABLE IF NOT EXISTS component_publications (
    source_id UUID PRIMARY KEY REFERENCES component_sources(id),
    digest TEXT NOT NULL REFERENCES component_versions(digest),
    published_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE TABLE IF NOT EXISTS component_installations (
    tenant_id TEXT NOT NULL,
    source_id UUID NOT NULL REFERENCES component_sources(id),
    digest TEXT NOT NULL REFERENCES component_versions(digest),
    enabled BOOLEAN NOT NULL DEFAULT true,
    generation UUID NOT NULL,
    PRIMARY KEY(tenant_id, source_id)
);
ALTER TABLE component_versions ADD COLUMN IF NOT EXISTS capabilities JSONB NOT NULL DEFAULT '{}';
CREATE TABLE IF NOT EXISTS component_process_keys (
    source_id UUID NOT NULL,
    tenant_id TEXT NOT NULL,
    ciphertext BYTEA NOT NULL,
    PRIMARY KEY(source_id,tenant_id)
);
CREATE TABLE IF NOT EXISTS component_process_actors (
    source_id UUID NOT NULL,
    tenant_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    PRIMARY KEY(source_id,tenant_id,user_id)
);
