CREATE TABLE IF NOT EXISTS pulse_visitors (
    site TEXT NOT NULL,
    user_stats_id BYTEA NOT NULL,
    first_seen_ts TIMESTAMPTZ NOT NULL,
    last_seen_ts TIMESTAMPTZ NOT NULL,
    session_start_ts TIMESTAMPTZ NOT NULL,
    entry_source_type TEXT,
    entry_ref_host TEXT,
    PRIMARY KEY (site, user_stats_id)
);

CREATE INDEX IF NOT EXISTS idx_pulse_visitors_site_last_seen
    ON pulse_visitors (site, last_seen_ts);
