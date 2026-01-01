ALTER TABLE pulse_events
    ADD COLUMN IF NOT EXISTS session_start_ts TIMESTAMPTZ;

UPDATE pulse_events
SET session_start_ts = ts
WHERE session_start_ts IS NULL;
