CREATE TABLE IF NOT EXISTS worker_devices (
 id TEXT PRIMARY KEY, token_hash TEXT NOT NULL UNIQUE, pairing_code TEXT UNIQUE,
 label TEXT NOT NULL, platform TEXT NOT NULL, capabilities JSONB NOT NULL,
 tenant_id TEXT, user_id TEXT, state TEXT NOT NULL DEFAULT 'pending',
 expires_at TIMESTAMPTZ NOT NULL DEFAULT now()+interval '10 minutes',
 last_seen TIMESTAMPTZ, created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS worker_device_owners ON worker_devices(tenant_id,user_id);
CREATE TABLE IF NOT EXISTS worker_tasks (
 id TEXT PRIMARY KEY, worker_id TEXT NOT NULL REFERENCES worker_devices(id),
 tenant_id TEXT NOT NULL, user_id TEXT NOT NULL, capability TEXT NOT NULL,
 input JSONB NOT NULL, state TEXT NOT NULL DEFAULT 'queued',
 result JSONB, error TEXT, lease TEXT, lease_until TIMESTAMPTZ,
 created_at TIMESTAMPTZ NOT NULL DEFAULT now(), completed_at TIMESTAMPTZ
);
CREATE INDEX IF NOT EXISTS worker_task_queue ON worker_tasks(worker_id,state,created_at);
CREATE TABLE IF NOT EXISTS worker_vaults (
 tenant_id TEXT NOT NULL,user_id TEXT NOT NULL,ciphertext BYTEA NOT NULL,
 PRIMARY KEY(tenant_id,user_id)
);
