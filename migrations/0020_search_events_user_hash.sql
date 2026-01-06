ALTER TABLE search_events
    ADD COLUMN IF NOT EXISTS search_user_hash TEXT;
