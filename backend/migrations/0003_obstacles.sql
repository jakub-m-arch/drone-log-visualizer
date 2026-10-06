-- Obstacle data fetched for a flight's area (OSM trees, GUGiK LiDAR), cached
-- so the external services are asked once per flight and source.
CREATE TABLE flight_obstacles (
    flight_id  INTEGER NOT NULL REFERENCES flights (id) ON DELETE CASCADE,
    source     TEXT    NOT NULL,
    geojson    TEXT    NOT NULL,
    fetched_at TEXT    NOT NULL,
    PRIMARY KEY (flight_id, source)
) WITHOUT ROWID;
