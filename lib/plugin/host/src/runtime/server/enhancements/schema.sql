CREATE TABLE IF NOT EXISTS runtime_enhancement_revisions (
    tenant_id TEXT NOT NULL,
    revision TEXT NOT NULL,
    package JSONB NOT NULL,
    author TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, revision)
);
CREATE TABLE IF NOT EXISTS runtime_enhancement_heads (
    tenant_id TEXT PRIMARY KEY,
    revision TEXT,
    generation BIGINT NOT NULL DEFAULT 0,
    FOREIGN KEY (tenant_id, revision) REFERENCES runtime_enhancement_revisions(tenant_id, revision)
);
CREATE TABLE IF NOT EXISTS runtime_enhancement_events (
    tenant_id TEXT NOT NULL,
    generation BIGINT NOT NULL,
    revision TEXT,
    actor TEXT NOT NULL,
    action TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, generation),
    FOREIGN KEY (tenant_id, revision) REFERENCES runtime_enhancement_revisions(tenant_id, revision)
);
CREATE TABLE IF NOT EXISTS runtime_enhancement_observations (
    tenant_id TEXT NOT NULL,
    revision TEXT NOT NULL,
    generation BIGINT NOT NULL,
    path TEXT NOT NULL,
    started BIGINT NOT NULL DEFAULT 0,
    succeeded BIGINT NOT NULL DEFAULT 0,
    failed BIGINT NOT NULL DEFAULT 0,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, generation, path),
    FOREIGN KEY (tenant_id, generation) REFERENCES runtime_enhancement_events(tenant_id, generation),
    FOREIGN KEY (tenant_id, revision) REFERENCES runtime_enhancement_revisions(tenant_id, revision)
);
CREATE INDEX IF NOT EXISTS runtime_enhancement_event_revision
    ON runtime_enhancement_events(tenant_id, revision);
