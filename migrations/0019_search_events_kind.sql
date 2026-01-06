ALTER TABLE search_events
    ADD COLUMN IF NOT EXISTS kind TEXT NOT NULL DEFAULT 'search';

CREATE INDEX IF NOT EXISTS idx_search_events_day_kind
    ON search_events (day, kind);
