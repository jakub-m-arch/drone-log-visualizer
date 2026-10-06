-- Gimbal and camera telemetry. Flights imported before this migration have
-- zeros here until they are re-imported (see `parse_version`).
ALTER TABLE samples ADD COLUMN gimbal_pitch_deg REAL NOT NULL DEFAULT 0;
ALTER TABLE samples ADD COLUMN is_photo INTEGER NOT NULL DEFAULT 0;
ALTER TABLE samples ADD COLUMN is_recording INTEGER NOT NULL DEFAULT 0;

-- Version of the parse/mapping code that produced a flight's rows. Uploading
-- the same file again re-parses flights with an older version in place.
ALTER TABLE flights ADD COLUMN parse_version INTEGER NOT NULL DEFAULT 1;
