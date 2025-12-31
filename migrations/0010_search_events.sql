CREATE TABLE IF NOT EXISTS search_events (
    id BIGSERIAL PRIMARY KEY,
    query_raw TEXT NOT NULL,
    query_norm TEXT NOT NULL,
    keyword_count INTEGER NOT NULL,
    tags TEXT[] NOT NULL DEFAULT '{}',
    category TEXT,
    range_start DATE,
    range_end DATE,
    sort TEXT NOT NULL,
    result_total INTEGER NOT NULL,
    elapsed_ms INTEGER NOT NULL,
    ts TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    day DATE GENERATED ALWAYS AS ((ts AT TIME ZONE 'UTC')::date) STORED
);

CREATE INDEX IF NOT EXISTS idx_search_events_day ON search_events (day);
CREATE INDEX IF NOT EXISTS idx_search_events_query_norm ON search_events (query_norm);
