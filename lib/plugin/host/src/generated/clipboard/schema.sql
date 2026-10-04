CREATE TABLE IF NOT EXISTS clipboard_heads (
    tenant_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    revision BIGINT NOT NULL DEFAULT 0,
    PRIMARY KEY(tenant_id,user_id)
);
CREATE TABLE IF NOT EXISTS clipboard_items (
    tenant_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    seq BIGINT NOT NULL,
    id TEXT NOT NULL,
    kind TEXT NOT NULL,
    mime TEXT NOT NULL,
    name TEXT,
    size BIGINT NOT NULL,
    hash TEXT NOT NULL,
    origin_device TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY(tenant_id,user_id,seq)
);
CREATE INDEX IF NOT EXISTS clipboard_item_id ON clipboard_items(tenant_id,user_id,id);
CREATE TABLE IF NOT EXISTS clipboard_chunks (
    tenant_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    seq BIGINT NOT NULL,
    idx INTEGER NOT NULL,
    ciphertext BYTEA NOT NULL,
    PRIMARY KEY(tenant_id,user_id,seq,idx)
);
