CREATE TABLE IF NOT EXISTS pulse_events (
    page_instance_id UUID PRIMARY KEY,
    duration_ms BIGINT,
    user_stats_id BYTEA,
    path TEXT,
    ts TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    ua_family TEXT,
    device TEXT,
    source_type TEXT,
    ref_host TEXT,
    country TEXT,
    day DATE GENERATED ALWAYS AS ((ts AT TIME ZONE 'UTC')::date) STORED
);

CREATE INDEX ON pulse_events (day);
CREATE INDEX ON pulse_events (day, path);
CREATE INDEX ON pulse_events (day, path, user_stats_id);
CREATE INDEX ON pulse_events (day, source_type);
