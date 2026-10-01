CREATE TABLE IF NOT EXISTS worker_devices (
 id TEXT PRIMARY KEY, token_hash TEXT NOT NULL UNIQUE, pairing_code TEXT UNIQUE,
 label TEXT NOT NULL, platform TEXT NOT NULL, capabilities JSONB NOT NULL,
 tenant_id TEXT, user_id TEXT, state TEXT NOT NULL DEFAULT 'pending',
 expires_at TIMESTAMPTZ NOT NULL DEFAULT now()+interval '10 minutes',
 last_seen TIMESTAMPTZ, created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS worker_device_owners ON worker_devices(tenant_id,user_id);
ALTER TABLE worker_devices ADD COLUMN IF NOT EXISTS machine_id TEXT;
CREATE UNIQUE INDEX IF NOT EXISTS worker_device_machine ON worker_devices(tenant_id,user_id,machine_id) WHERE state='active' AND machine_id IS NOT NULL;
CREATE TABLE IF NOT EXISTS worker_tasks (
 id TEXT PRIMARY KEY, worker_id TEXT NOT NULL REFERENCES worker_devices(id),
 tenant_id TEXT NOT NULL, user_id TEXT NOT NULL, capability TEXT NOT NULL,
 input JSONB NOT NULL, state TEXT NOT NULL DEFAULT 'queued',
 result JSONB, error TEXT, lease TEXT, lease_until TIMESTAMPTZ,
 created_at TIMESTAMPTZ NOT NULL DEFAULT now(), completed_at TIMESTAMPTZ
);
CREATE INDEX IF NOT EXISTS worker_task_queue ON worker_tasks(worker_id,state,created_at);
ALTER TABLE worker_tasks ADD COLUMN IF NOT EXISTS claim_id TEXT;
CREATE UNIQUE INDEX IF NOT EXISTS worker_task_claim ON worker_tasks(worker_id,claim_id) WHERE claim_id IS NOT NULL;
CREATE TABLE IF NOT EXISTS worker_terminal_sessions (
 id TEXT PRIMARY KEY, worker_id TEXT NOT NULL REFERENCES worker_devices(id) ON DELETE CASCADE,
 tenant_id TEXT NOT NULL, user_id TEXT NOT NULL, state TEXT NOT NULL DEFAULT 'waiting',
 cols INTEGER NOT NULL, rows INTEGER NOT NULL, browser_cursor BIGINT NOT NULL DEFAULT 0,
 device_cursor BIGINT NOT NULL DEFAULT 0, lease TEXT, lease_until TIMESTAMPTZ,
 created_at TIMESTAMPTZ NOT NULL DEFAULT now(), updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
 closed_at TIMESTAMPTZ
);
CREATE INDEX IF NOT EXISTS worker_terminal_queue ON worker_terminal_sessions(worker_id,state,created_at);
CREATE INDEX IF NOT EXISTS worker_terminal_expiry ON worker_terminal_sessions(worker_id,updated_at);
CREATE TABLE IF NOT EXISTS worker_terminal_frames (
 session_id TEXT NOT NULL REFERENCES worker_terminal_sessions(id) ON DELETE CASCADE,
 direction TEXT NOT NULL, cursor BIGINT NOT NULL, kind TEXT NOT NULL, data TEXT NOT NULL,
 created_at TIMESTAMPTZ NOT NULL DEFAULT now(), PRIMARY KEY(session_id,direction,cursor)
);
CREATE INDEX IF NOT EXISTS worker_terminal_frames_cursor ON worker_terminal_frames(session_id,direction,cursor);
CREATE TABLE IF NOT EXISTS worker_vaults (
 tenant_id TEXT NOT NULL,user_id TEXT NOT NULL,ciphertext BYTEA NOT NULL,
 PRIMARY KEY(tenant_id,user_id)
);
