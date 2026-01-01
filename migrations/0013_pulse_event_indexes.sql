DROP INDEX IF EXISTS pulse_events_day_idx;
DROP INDEX IF EXISTS pulse_events_day_path_idx;
DROP INDEX IF EXISTS pulse_events_day_path_user_stats_id_idx;
DROP INDEX IF EXISTS pulse_events_day_source_type_idx;
DROP INDEX IF EXISTS idx_pulse_events_day_site;

CREATE INDEX IF NOT EXISTS idx_pulse_events_site_day_path
    ON pulse_events (site, day, path);

CREATE INDEX IF NOT EXISTS idx_pulse_events_site_day_source_type
    ON pulse_events (site, day, source_type);

CREATE INDEX IF NOT EXISTS idx_pulse_events_site_day_user_stats_id
    ON pulse_events (site, day, user_stats_id);
