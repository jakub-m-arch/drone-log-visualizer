CREATE TABLE flights (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    file_name       TEXT    NOT NULL,
    file_sha256     TEXT    NOT NULL UNIQUE,
    file_size       INTEGER NOT NULL,
    uploaded_at     TEXT    NOT NULL,
    log_version     INTEGER NOT NULL,
    encrypted       INTEGER NOT NULL,
    aircraft_name   TEXT    NOT NULL,
    aircraft_sn     TEXT    NOT NULL,
    product_type    TEXT    NOT NULL,
    app_platform    TEXT    NOT NULL,
    app_version     TEXT    NOT NULL,
    start_time      TEXT,
    duration_s      REAL    NOT NULL,
    distance_m      REAL    NOT NULL,
    max_height_m    REAL    NOT NULL,
    max_h_speed_ms  REAL    NOT NULL,
    max_v_speed_ms  REAL    NOT NULL,
    home_lat        REAL,
    home_lon        REAL,
    takeoff_lat     REAL,
    takeoff_lon     REAL,
    landing_lat     REAL,
    landing_lon     REAL,
    location        TEXT    NOT NULL,
    sample_count    INTEGER NOT NULL
);

CREATE INDEX flights_start_time ON flights (start_time);

CREATE TABLE samples (
    flight_id       INTEGER NOT NULL REFERENCES flights (id) ON DELETE CASCADE,
    idx             INTEGER NOT NULL,
    t               REAL    NOT NULL,
    timestamp_ms    INTEGER,
    lat             REAL,
    lon             REAL,
    height_m        REAL    NOT NULL,
    altitude_m      REAL    NOT NULL,
    h_speed_ms      REAL    NOT NULL,
    v_speed_ms      REAL    NOT NULL,
    yaw_deg         REAL    NOT NULL,
    pitch_deg       REAL    NOT NULL,
    roll_deg        REAL    NOT NULL,
    battery_pct     REAL,
    battery_v       REAL,
    gps_sats        INTEGER NOT NULL,
    rc_uplink_pct   REAL,
    rc_downlink_pct REAL,
    flight_mode     TEXT    NOT NULL,
    is_flying       INTEGER NOT NULL,
    PRIMARY KEY (flight_id, idx)
) WITHOUT ROWID;

CREATE TABLE events (
    flight_id INTEGER NOT NULL REFERENCES flights (id) ON DELETE CASCADE,
    idx       INTEGER NOT NULL,
    t         REAL    NOT NULL,
    level     TEXT    NOT NULL,
    message   TEXT    NOT NULL,
    PRIMARY KEY (flight_id, idx)
) WITHOUT ROWID;

-- Decryption keys returned by the DJI API, keyed by a hash of the keychain
-- request (which is derived from the log), so the same log never needs a
-- second API call.
CREATE TABLE keychain_cache (
    request_sha256 TEXT PRIMARY KEY,
    keychains_json TEXT NOT NULL,
    created_at     TEXT NOT NULL
);
