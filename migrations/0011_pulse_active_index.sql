CREATE INDEX IF NOT EXISTS idx_pulse_events_site_ts
    ON pulse_events (site, ts);
