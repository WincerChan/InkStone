UPDATE pulse_events
SET site = 'unknown'
WHERE site IS NULL OR site = '';

ALTER TABLE pulse_events
    ALTER COLUMN site SET NOT NULL;

ALTER TABLE pulse_events
    RENAME COLUMN source_type TO entry_source_type;

ALTER TABLE pulse_events
    RENAME COLUMN ref_host TO entry_ref_host;

DROP INDEX IF EXISTS idx_pulse_events_site_day_source_type;

CREATE INDEX IF NOT EXISTS idx_pulse_events_site_day_entry_source_type
    ON pulse_events (site, day, entry_source_type);

CREATE INDEX IF NOT EXISTS idx_pulse_events_site_day_session
    ON pulse_events (site, day, user_stats_id, session_start_ts);
