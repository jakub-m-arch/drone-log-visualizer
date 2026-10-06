export interface AppConfig {
  appVersion: string
  parserVersion: string
  apiKeyConfigured: boolean
  supportedLogVersions: { min: number; max: number }
  encryptedFromVersion: number
  maxUploadMb: number
  mapTileUrl: string
  mapAttribution: string
  /** Raster DEM tile URL for the 3D view, or null for flat 3D. */
  terrainUrl: string | null
  terrainEncoding: 'terrarium' | 'mapbox'
  terrainAttribution: string
  /** TileJSON URL of OpenMapTiles vector tiles for 3D buildings/woods, or null. */
  vectorTilesUrl: string | null
  /** Which on-demand obstacle sources the server has enabled. */
  obstacleSources: { trees: boolean; lidar: boolean }
}

export interface FlightSummary {
  id: number
  fileName: string
  fileSize: number
  uploadedAt: string
  logVersion: number
  encrypted: boolean
  aircraftName: string
  aircraftSn: string
  productType: string
  appPlatform: string
  appVersion: string
  startTime: string | null
  durationS: number
  distanceM: number
  maxHeightM: number
  maxHSpeedMs: number
  maxVSpeedMs: number
  homeLat: number | null
  homeLon: number | null
  takeoffLat: number | null
  takeoffLon: number | null
  landingLat: number | null
  landingLon: number | null
  location: string
  sampleCount: number
}

export interface FlightEvent {
  t: number
  level: 'info' | 'warning'
  message: string
}

export interface FlightDetail {
  flight: FlightSummary
  events: FlightEvent[]
}

/** Column-oriented telemetry: one array per series, all the same length. */
export interface Telemetry {
  t: number[]
  timestampMs: (number | null)[]
  lat: (number | null)[]
  lon: (number | null)[]
  heightM: number[]
  altitudeM: number[]
  hSpeedMs: number[]
  vSpeedMs: number[]
  yawDeg: number[]
  batteryPct: (number | null)[]
  batteryV: (number | null)[]
  gpsSats: number[]
  rcUplinkPct: (number | null)[]
  rcDownlinkPct: (number | null)[]
  flightMode: string[]
  isFlying: boolean[]
  gimbalPitchDeg: number[]
  isPhoto: boolean[]
  isRecording: boolean[]
}

export type ObstacleSource = 'trees' | 'lidar'

export interface ObstacleResponse {
  source: ObstacleSource
  fetchedAt: string
  cached: boolean
  data: {
    type: 'FeatureCollection'
    features: {
      type: 'Feature'
      properties: { height: number | null; crown?: number | null; groundRel?: number }
      geometry: { type: 'Point'; coordinates: [number, number] } | { type: 'Polygon'; coordinates: [number, number][][] }
    }[]
    /** LiDAR only: number of tiles fetched, and whether the corridor was cut. */
    tiles?: number
    truncated?: boolean
  }
}

/** LiDAR is fetched in the background; the API answers 202 until it is done. */
export interface ObstacleLoading {
  source: ObstacleSource
  status: 'loading'
  done: number
  total: number
}

export function isLoading(r: ObstacleResponse | ObstacleLoading): r is ObstacleLoading {
  return (r as ObstacleLoading).status === 'loading'
}

export interface UploadResult {
  created: boolean
  /** An already imported flight was refreshed by a newer parser version. */
  reparsed: boolean
  flight: FlightSummary
}

/** Error returned by the backend: `{ error: { code, message } }`. */
export class ApiError extends Error {
  constructor(
    public code: string,
    message: string,
    public status: number,
  ) {
    super(message)
  }
}

async function parseError(res: Response): Promise<ApiError> {
  try {
    const body = await res.json()
    if (body?.error?.code) return new ApiError(body.error.code, body.error.message, res.status)
  } catch {
    /* not JSON */
  }
  return new ApiError('http_error', `Request failed (HTTP ${res.status})`, res.status)
}

async function request<T>(url: string, init?: RequestInit): Promise<T> {
  let res: Response
  try {
    res = await fetch(url, init)
  } catch {
    throw new ApiError('offline', 'Cannot reach the server.', 0)
  }
  if (!res.ok) throw await parseError(res)
  if (res.status === 204) return undefined as T
  return res.json() as Promise<T>
}

export const api = {
  config: () => request<AppConfig>('/api/config'),
  flights: () => request<FlightSummary[]>('/api/flights'),
  flight: (id: number) => request<FlightDetail>(`/api/flights/${id}`),
  telemetry: (id: number) => request<Telemetry>(`/api/flights/${id}/telemetry`),
  deleteFlight: (id: number) => request<void>(`/api/flights/${id}`, { method: 'DELETE' }),
  obstacles: (id: number, source: ObstacleSource, refresh = false) =>
    request<ObstacleResponse | ObstacleLoading>(
      `/api/flights/${id}/obstacles/${source}${refresh ? '?refresh=true' : ''}`,
    ),
  exportUrl: (id: number, format: 'csv' | 'gpx' | 'kml') => `/api/flights/${id}/export/${format}`,

  /** Uploads a log with progress reporting (fetch has no upload progress). */
  upload(file: File, onProgress?: (fraction: number) => void): Promise<UploadResult> {
    return new Promise((resolve, reject) => {
      const xhr = new XMLHttpRequest()
      xhr.open('POST', '/api/flights')
      xhr.responseType = 'json'
      xhr.upload.onprogress = (e) => {
        if (e.lengthComputable && onProgress) onProgress(e.loaded / e.total)
      }
      xhr.onerror = () => reject(new ApiError('offline', 'Cannot reach the server.', 0))
      xhr.onload = () => {
        const body = xhr.response
        if (xhr.status >= 200 && xhr.status < 300) {
          resolve(body as UploadResult)
        } else if (body?.error?.code) {
          reject(new ApiError(body.error.code, body.error.message, xhr.status))
        } else {
          reject(new ApiError('http_error', `Upload failed (HTTP ${xhr.status})`, xhr.status))
        }
      }
      const form = new FormData()
      form.append('file', file)
      xhr.send(form)
    })
  },
}

/** Extra guidance shown under specific backend errors. */
export function errorHint(code: string): string | null {
  switch (code) {
    case 'missing_api_key':
      return 'Logs from recent DJI apps (format v13+) are encrypted. Create a free DJI Developer app, then restart the container with DJI_API_KEY set (see README).'
    case 'invalid_api_key':
      return 'Make sure you copied the "App Key" of an activated DJI Developer app with the Open API type.'
    case 'unsupported_version':
      return 'Only DJIFlightRecord_*.txt files from DJI Fly / DJI GO apps are supported.'
    case 'decryption_failed':
      return 'The keys returned by DJI did not match this log. Try again later; if it persists, the log may be damaged.'
    case 'network_error':
    case 'dji_api_error':
      return 'Decrypting v13+ logs requires internet access to the DJI API from the server.'
    case 'too_large':
      return 'Increase MAX_UPLOAD_MB in the container environment.'
    default:
      return null
  }
}
