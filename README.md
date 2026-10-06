# DJI Flight Log Viewer

Self-hosted web app for exploring DJI flight logs (`DJIFlightRecord_*.txt`).
Runs locally as a single Docker container; your logs never leave your machine
(except for the decryption-key request described [below](#what-is-sent-to-dji)).

- Drag & drop upload of single logs or a whole `FlightRecord` folder (subfolders
  included), with batch progress and errors grouped by cause; re-imports are
  de-duplicated by content
- Flight list with totals (flights, flight time, distance, longest flight, max
  height, period) and a per-aircraft breakdown; filter by aircraft, date range
  and text, sort by date, duration, distance or height
- Flight view
  - track on an OpenStreetMap map with take-off, landing and home point,
    optionally colored by height, speed or battery; photo positions and
    video-recording sections are marked
  - 3D view: the track as a ribbon at its real height with a curtain down to
    the ground, over terrain (free AWS Terrain Tiles, no key) or flat ground,
    with 3D buildings and woods from OpenStreetMap (free OpenFreeMap tiles);
    on demand, single trees from OSM and — for flights in Poland — real object
    heights from GUGiK airborne laser scanning
  - timeline slider and playback (1×–50×) with the aircraft moving along the track
  - synchronized charts: height, horizontal/vertical speed, battery (% and V),
    GPS satellites, gimbal pitch, RC signal — drag to zoom, click to jump;
    legends show the values at the playhead
  - list of app warnings and events (take-off/landing, photos, video
    start/stop), click to jump
- Export to CSV, GPX and KML
- Clear errors for missing/invalid DJI key and unsupported log versions

Stack: Rust ([axum](https://github.com/tokio-rs/axum),
[`dji-log-parser`](https://github.com/lvauvillier/dji-log-parser), SQLite via sqlx)
and Svelte + TypeScript ([MapLibre GL](https://maplibre.org/), [uPlot](https://github.com/leeoniya/uPlot)).

## Quick start

Requires Docker with the Compose plugin.

```sh
git clone https://github.com/jakub-m-arch/drone-log-visualizer.git
cd drone-log-visualizer
cp .env.example .env        # optional: add your DJI_API_KEY (see below)
docker compose up -d --build
```

Open <http://localhost:8080> and drop a flight log onto the page.

Data (flights, telemetry, cached decryption keys) is stored in the `dji-data`
Docker volume, mounted at `/data`. `docker compose down` keeps it,
`docker compose down -v` deletes it.

To update: `git pull && docker compose up -d --build`.

## Where to find your flight logs

| App | Location |
| --- | --- |
| DJI Fly (Android) | `Android/data/dji.go.v5/files/FlightRecord/` |
| DJI Fly (iOS) | Finder / iTunes → File Sharing → DJI Fly → `FlightRecords` |
| DJI GO 4 (Android) | `DJI/dji.go.v4/FlightRecord/` |
| DJI RC / RC Pro (screen controllers) | connect via USB → `Android/data/dji.go.v5/files/FlightRecord/` |

Paths can differ between app versions.

## DJI API key (for encrypted logs)

Logs written by current DJI apps (log format **v13 and newer**) are encrypted.
The keys to decrypt a log are issued by DJI's Open API for that specific log,
so the viewer needs a DJI developer **App Key**. Older logs (v1–v12) work without one.

1. Sign in at [developer.dji.com](https://developer.dji.com/user) (a regular DJI account works).
2. Click **Create App**, choose **Open API** as the app type, and fill in name,
   category and description.
3. Activate the app via the link DJI sends by e-mail.
4. On your developer page open the app and copy its **App Key**.
5. Put it in `.env` next to `docker-compose.yml`:

   ```sh
   DJI_API_KEY=your-app-key
   ```

6. Restart: `docker compose up -d`.

The banner on the start page disappears once a key is configured.

### Using your own key

Each user of this project runs it with **their own** DJI developer account and
App Key, and is responsible for using it in line with the
DJI Developer terms shown in your account on [developer.dji.com](https://developer.dji.com/) that apply to their
account. This project does not ship, share or proxy any key.

The key is only read from the environment. It is never written to the
database, to application logs, or to API responses (`/api/config` only says
whether a key is configured). Keep `.env` out of version control — it is already
in `.gitignore`.

### What is sent to DJI

For an encrypted log, the server sends DJI the log's encrypted key records
(`keychainsArray`) plus the log's app version/department — the same request
DJI's own tooling makes. Telemetry, GPS positions and the log file itself are
not sent. The returned keys are cached in SQLite (keyed by a hash of that
request), so re-importing the same log does not contact DJI again.

## Configuration

Set in `.env` (read by `docker compose`) or as container environment variables.

| Variable | Default | Description |
| --- | --- | --- |
| `DJI_API_KEY` | – | DJI Open API App Key, needed for v13+ logs |
| `PORT` | `8080` | HTTP port. In `docker-compose.yml` it is the host port; the container always listens on 8080 |
| `DATA_DIR` | `/data` | SQLite database location |
| `MAX_UPLOAD_MB` | `200` | Maximum size of an uploaded log |
| `MAP_TILE_URL` | OSM | Raster tile URL template (`{z}/{x}/{y}`) |
| `MAP_ATTRIBUTION` | OSM | Attribution HTML shown on the map |
| `MAP_TERRAIN_URL` | AWS Terrain Tiles | Raster DEM tiles for the 3D view; `off` = flat 3D |
| `MAP_TERRAIN_ENCODING` | `terrarium` | `terrarium` or `mapbox` (Terrain-RGB) |
| `MAP_TERRAIN_ATTRIBUTION` | Mapzen | Attribution HTML for the elevation data |
| `MAP_VECTOR_TILES_URL` | OpenFreeMap | TileJSON of OpenMapTiles-schema vector tiles for 3D buildings and woods; `off` disables them |
| `OVERPASS_URL` | overpass-api.de + 2 mirrors | Overpass API endpoints for OSM trees, comma-separated, tried in order when one is busy; `off` disables the layer |
| `GUGIK_NMT_URL` / `GUGIK_NMPT_URL` | GUGiK WCS | GeoTIFF request templates for terrain (NMT) and surface (NMPT) models, with `{minE} {minN} {maxE} {maxN}` in EPSG:2180; `off` disables the LiDAR layer |
| `RUST_LOG` | `info` | Log level |

The default map uses the public OpenStreetMap tile servers, which is fine for
personal use under the [OSM tile usage policy](https://operations.osmfoundation.org/policies/tiles/).
Point `MAP_TILE_URL` at another provider or your own tile server for heavier use.

The 3D view loads elevation from the public
[AWS Terrain Tiles](https://registry.opendata.aws/terrain-tiles/) dataset
(Terrarium encoding, no key). Heights in DJI logs are relative to the take-off
point, so the track is placed relative to the terrain at take-off; terrain
resolution (~10–30 m) limits accuracy close to the ground.

3D buildings and woods come from [OpenFreeMap](https://openfreemap.org/)
vector tiles (OpenMapTiles schema, OpenStreetMap data, no key). Building
heights are OSM `height` / `building:levels` where mapped; many buildings
have no height in OSM and get a 6 m default. Woods have no height data and are
drawn at an assumed 15 m canopy. Individual trees and power lines are not part
of these tiles. Treat all of this as context, not as obstacle clearance data.

Two more 3D layers are fetched **only when you switch them on** for a flight,
by the server, for the flight's area within 500 m of take-off, and then
cached in SQLite:

- **Trees (OSM):** `natural=tree` points from the
  [Overpass API](https://wiki.openstreetmap.org/wiki/Overpass_API), drawn with
  the mapped `height` / `diameter_crown` or a 10 m / 5 m default. Coverage
  differs a lot between places.
- **LiDAR heights (GUGiK), Poland only:** the national terrain (NMT, 1 m)
  and surface (NMPT, 0.5 m) models from airborne laser scanning, published
  free of charge by GUGiK. Their difference is the real height of every tree,
  building and other object above ground — the layer shows *all* of them,
  coloured by height (low = green, tall = brown). Objects ≥ 2.5 m are drawn
  as 1 m blocks. Only the part of the track within 500 m of take-off is
  covered: 150 m tiles within 50 m of it (at most 12, nearest to take-off
  first). GUGiK often drops connections; requests are retried, and a tile
  that still fails is skipped (the legend says how many) so the rest is kept. GUGiK renders NMPT slowly — about
  1–3 minutes per tile — so the server fetches in the background, the legend
  shows the progress, and the result is cached per flight; the ↻ button
  fetches it again. Coordinates are converted to PUWG 1992 (EPSG:2180);
  grids that do not cover the requested area are rejected. Verified against
  the Palace of Culture in Warsaw (234 m above ground at the spire). The
  scans are several years old and often made without leaves, so recent or
  deciduous trees can be missing or lower.

Both requests reveal the flight's area to the respective service.

## Supported logs and errors

Parsing is done by [`dji-log-parser`](https://crates.io/crates/dji-log-parser) 0.5.7 (MIT),
which supports log formats **v1–v14**.

| Error | Meaning |
| --- | --- |
| `missing_api_key` | Encrypted (v13+) log and no `DJI_API_KEY` configured |
| `invalid_api_key` | DJI rejected the key (HTTP 401/403) — check the key and that the app is activated |
| `unsupported_version` | Log format newer than v14, or not a DJI flight log |
| `invalid_log` | File is damaged/truncated or not a flight log |
| `decryption_failed` | DJI returned keys that do not decrypt this log (the keys are not cached) |
| `network_error` / `dji_api_error` | DJI API unreachable or returned an error |

## Development

Backend (Rust ≥ 1.85):

```sh
cd backend
DATA_DIR=./data STATIC_DIR=../frontend/dist cargo run
cargo test                      # unit + API tests
cargo run --bin gen-sample-log -- DJIFlightRecord_sample.txt   # synthetic test log
```

Frontend (Node 22):

```sh
cd frontend
npm ci
npm run dev        # http://localhost:5173, proxies /api to localhost:8080
npm run check && npm test
```

When the mapping in `backend/src/flight.rs` starts extracting new data, bump
`PARSE_VERSION` in `backend/src/ingest.rs` (and add a migration if needed):
re-uploading a log that is already stored then re-parses it in place instead
of reporting a duplicate, so users get the new data by dropping their
`FlightRecord` folder again.

Real flight logs contain precise locations and serial numbers, so the tests use
synthetic logs (plain v6 and AES-encrypted v14) generated by
`backend/src/synthetic.rs`, and a mock of the DJI keychain endpoint.
`gen-sample-log` writes such a file so you can try the UI without a drone.

### Layout

```
backend/    Rust API server (axum), parser mapping, SQLite (migrations/), exports
frontend/   Svelte + TypeScript UI (Vite)
Dockerfile  frontend build → backend build → slim runtime image
```

### HTTP API

| Method | Path | |
| --- | --- | --- |
| `GET` | `/api/config` | versions, whether a key is configured, map settings |
| `GET` | `/api/flights` | flight list |
| `POST` | `/api/flights` | upload (`multipart/form-data`, field `file`) |
| `GET` | `/api/flights/{id}` | flight summary and events |
| `GET` | `/api/flights/{id}/telemetry` | column-oriented telemetry |
| `GET` | `/api/flights/{id}/export/{csv,gpx,kml}` | export |
| `DELETE` | `/api/flights/{id}` | delete flight |

Errors are returned as `{"error": {"code": "...", "message": "..."}}`.

## Roadmap

Not in the MVP: 3D view (deck.gl / CesiumJS), video sync, fleet statistics,
multiple users.

## License

[MIT](LICENSE). Not affiliated with or endorsed by DJI. "DJI" is a trademark of
SZ DJI Technology Co., Ltd.
