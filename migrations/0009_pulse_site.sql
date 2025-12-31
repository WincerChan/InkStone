ALTER TABLE pulse_events
    ADD COLUMN IF NOT EXISTS site TEXT;

CREATE INDEX IF NOT EXISTS idx_pulse_events_day_site
    ON pulse_events (day, site);
