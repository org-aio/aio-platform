SELECT pg_advisory_xact_lock(72109414);
CREATE TABLE IF NOT EXISTS marketplace_source_archive (
    kind TEXT NOT NULL,
    identity TEXT NOT NULL,
    record JSONB NOT NULL,
    archived_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (kind, identity)
);
DO $$ BEGIN
    IF to_regclass('plugin_registries') IS NOT NULL THEN
        INSERT INTO marketplace_source_archive(kind, identity, record)
        SELECT 'registry', id, to_jsonb(source) FROM plugin_registries source
        ON CONFLICT DO NOTHING;
        DROP TABLE plugin_registries;
    END IF;
    IF to_regclass('marketplace_registry_syncs') IS NOT NULL THEN
        INSERT INTO marketplace_source_archive(kind, identity, record)
        SELECT 'sync', source, to_jsonb(sync) FROM marketplace_registry_syncs sync
        ON CONFLICT DO NOTHING;
        DROP TABLE marketplace_registry_syncs;
    END IF;
END $$;
INSERT INTO marketplace_source_archive(kind, identity, record)
SELECT 'entry', jsonb_build_array(source, git)::text, to_jsonb(entry)
FROM marketplace_entries entry WHERE source <> 'aio://published'
ON CONFLICT DO NOTHING;
DELETE FROM marketplace_entries WHERE source <> 'aio://published';
