CREATE TABLE IF NOT EXISTS delivery_sources (
    git TEXT PRIMARY KEY,
    branch TEXT NOT NULL,
    desired_sha TEXT NOT NULL,
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE TABLE IF NOT EXISTS delivery_jobs (
    id BIGSERIAL PRIMARY KEY,
    git TEXT NOT NULL REFERENCES delivery_sources(git),
    source_revision TEXT NOT NULL,
    recipe JSONB NOT NULL,
    state TEXT NOT NULL DEFAULT 'queued',
    lease TEXT,
    lease_until TIMESTAMPTZ,
    package_revision TEXT,
    error TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(git,source_revision)
);
CREATE TABLE IF NOT EXISTS plugin_documents (
    revision TEXT PRIMARY KEY REFERENCES plugin_packages(revision) ON DELETE CASCADE,
    readme TEXT NOT NULL,
    images JSONB NOT NULL DEFAULT '{}'
);
ALTER TABLE delivery_jobs ADD COLUMN IF NOT EXISTS retry_count INTEGER NOT NULL DEFAULT 0;
ALTER TABLE delivery_jobs ADD COLUMN IF NOT EXISTS next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT now();
CREATE TABLE IF NOT EXISTS delivery_installations (
    tenant_id TEXT NOT NULL,
    source_id TEXT NOT NULL REFERENCES plugin_sources(id) ON DELETE CASCADE ON UPDATE CASCADE,
    excluded_revision TEXT,
    PRIMARY KEY(tenant_id,source_id)
);
INSERT INTO delivery_installations(tenant_id,source_id)
SELECT tenant_id,source_id FROM tenant_plugin_bindings ON CONFLICT DO NOTHING;
CREATE TABLE IF NOT EXISTS delivery_rollouts (
    tenant_id TEXT NOT NULL,
    source_id TEXT NOT NULL REFERENCES plugin_sources(id) ON DELETE CASCADE ON UPDATE CASCADE,
    revision TEXT NOT NULL,
    state TEXT NOT NULL DEFAULT 'queued',
    error TEXT,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY(tenant_id,source_id,revision)
);
