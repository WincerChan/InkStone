ALTER TABLE pulse_visitors
  ADD COLUMN IF NOT EXISTS last_device TEXT;

ALTER TABLE pulse_visitors
  ADD COLUMN IF NOT EXISTS last_ua_family TEXT;

ALTER TABLE pulse_visitors
  ADD COLUMN IF NOT EXISTS last_country TEXT;
