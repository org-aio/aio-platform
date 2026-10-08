CREATE TABLE IF NOT EXISTS worker_webview_sessions (
 id TEXT PRIMARY KEY,
 worker_id TEXT NOT NULL REFERENCES worker_devices(id) ON DELETE CASCADE,
 tenant_id TEXT NOT NULL,
 user_id TEXT NOT NULL,
 session_id TEXT NOT NULL,
 source_id TEXT NOT NULL,
 revision TEXT NOT NULL,
 mount_digest TEXT NOT NULL,
 state TEXT NOT NULL DEFAULT 'waiting',
 expires_at TIMESTAMPTZ NOT NULL DEFAULT now() + interval '30 minutes',
 created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS worker_webview_owner ON worker_webview_sessions(tenant_id,user_id,mount_digest,state);
CREATE INDEX IF NOT EXISTS worker_webview_expiry ON worker_webview_sessions(expires_at);
