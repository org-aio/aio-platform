CREATE TABLE IF NOT EXISTS personal_config_heads (
    tenant_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    revision BIGINT NOT NULL DEFAULT 0,
    PRIMARY KEY(tenant_id,user_id)
);
CREATE TABLE IF NOT EXISTS personal_config_entries (
    tenant_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    id TEXT NOT NULL,
    kind TEXT NOT NULL,
    target TEXT NOT NULL,
    layer TEXT NOT NULL,
    format TEXT NOT NULL,
    secret BOOLEAN NOT NULL,
    executable BOOLEAN NOT NULL,
    deleted BOOLEAN NOT NULL,
    revision BIGINT NOT NULL,
    hash TEXT NOT NULL,
    size BIGINT NOT NULL,
    ciphertext BYTEA NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY(tenant_id,user_id,id),
    UNIQUE(tenant_id,user_id,kind,target,layer)
);
CREATE TABLE IF NOT EXISTS personal_config_history (
    tenant_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    entry_id TEXT NOT NULL,
    revision BIGINT NOT NULL,
    metadata JSONB NOT NULL,
    ciphertext BYTEA NOT NULL,
    PRIMARY KEY(tenant_id,user_id,entry_id,revision)
);
CREATE TABLE IF NOT EXISTS personal_config_devices (
    tenant_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    device_id TEXT NOT NULL REFERENCES worker_devices(id) ON DELETE CASCADE,
    report JSONB NOT NULL DEFAULT '{}'::jsonb,
    resolutions JSONB NOT NULL DEFAULT '{}'::jsonb,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY(tenant_id,user_id,device_id)
);
