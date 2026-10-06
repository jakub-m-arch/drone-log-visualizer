import type { Telemetry } from './api'
import type { LngLat } from './geo'

export type ColorMode = 'plain' | 'height' | 'speed' | 'battery'

export const colorModes: { value: ColorMode; label: string; unit: string }[] = [
  { value: 'plain', label: 'Plain', unit: '' },
  { value: 'height', label: 'Height', unit: 'm' },
  { value: 'speed', label: 'Speed', unit: 'm/s' },
  { value: 'battery', label: 'Battery', unit: '%' },
]

type Rgb = [number, number, number]

// Sequential ramp (low → high) and a red → green ramp for battery.
const RAMP: Rgb[] = [
  [49, 54, 149],
  [69, 117, 180],
  [116, 173, 209],
  [254, 224, 144],
  [244, 109, 67],
  [165, 0, 38],
]
const BATTERY_RAMP: Rgb[] = [
  [215, 48, 39],
  [252, 141, 89],
  [254, 224, 139],
  [145, 207, 96],
  [26, 152, 80],
]

export function rampFor(mode: ColorMode): Rgb[] {
  return mode === 'battery' ? BATTERY_RAMP : RAMP
}

/** Color at fraction `f` (0..1) of a ramp, as `rgb(r, g, b)`. */
export function rampColor(ramp: Rgb[], f: number): string {
  const x = Math.min(1, Math.max(0, Number.isFinite(f) ? f : 0)) * (ramp.length - 1)
  const i = Math.min(ramp.length - 2, Math.floor(x))
  const t = x - i
  const c = ramp[i].map((v, k) => Math.round(v + (ramp[i + 1][k] - v) * t))
  return `rgb(${c[0]}, ${c[1]}, ${c[2]})`
}

export function rampCss(mode: ColorMode): string {
  const ramp = rampFor(mode)
  return `linear-gradient(to right, ${ramp.map((_, i) => rampColor(ramp, i / (ramp.length - 1))).join(', ')})`
}

function series(tel: Telemetry, mode: ColorMode): (number | null)[] | null {
  switch (mode) {
    case 'height':
      return tel.heightM
    case 'speed':
      return tel.hSpeedMs
    case 'battery':
      return tel.batteryPct
    default:
      return null
  }
}

export interface ColoredTrack {
  features: {
    type: 'Feature'
    properties: { color: string }
    geometry: { type: 'LineString'; coordinates: LngLat[] }
  }[]
  min: number
  max: number
}

/**
 * Splits the track into at most `maxSegments` line segments, each colored by
 * the mean of the chosen series over its samples. Returns null for the plain
 * mode or when the series has no data.
 */
export function coloredTrack(
  tel: Telemetry,
  track: { idx: number[]; coords: LngLat[] },
  mode: ColorMode,
  maxSegments = 1500,
): ColoredTrack | null {
  const values = series(tel, mode)
  if (!values || track.coords.length < 2) return null
  const known = track.idx.map((i) => values[i]).filter((v): v is number => v !== null && Number.isFinite(v))
  if (!known.length) return null
  let min = Math.min(...known)
  let max = Math.max(...known)
  if (mode === 'battery') {
    min = Math.min(min, 0)
    max = 100
  }
  const span = max - min || 1
  const ramp = rampFor(mode)

  const n = track.coords.length
  const step = Math.max(1, Math.ceil((n - 1) / maxSegments))
  const features: ColoredTrack['features'] = []
  for (let a = 0; a < n - 1; a += step) {
    const b = Math.min(n - 1, a + step)
    let sum = 0
    let cnt = 0
    for (let k = a; k <= b; k++) {
      const v = values[track.idx[k]]
      if (v !== null && Number.isFinite(v)) {
        sum += v
        cnt++
      }
    }
    const color = cnt ? rampColor(ramp, (sum / cnt - min) / span) : 'rgb(140, 140, 140)'
    features.push({
      type: 'Feature',
      properties: { color },
      // Segments share their end points so the line has no gaps.
      geometry: { type: 'LineString', coordinates: track.coords.slice(a, b + 1) },
    })
  }
  return { features, min, max }
}

/** Track pieces where the camera was recording, as separate line strings. */
export function recordingSegments(tel: Telemetry, track: { idx: number[]; coords: LngLat[] }): LngLat[][] {
  const out: LngLat[][] = []
  let current: LngLat[] | null = null
  track.idx.forEach((i, k) => {
    if (tel.isRecording?.[i]) {
      if (!current) {
        current = []
        out.push(current)
      }
      current.push(track.coords[k])
    } else {
      current = null
    }
  })
  return out.filter((seg) => seg.length > 1)
}

/** Positions (and sample index) where a photo was taken: first sample of each run. */
export function photoPoints(tel: Telemetry, track: { idx: number[]; coords: LngLat[] }) {
  const pts: { coord: LngLat; sample: number }[] = []
  track.idx.forEach((i, k) => {
    if (tel.isPhoto?.[i] && !tel.isPhoto[i - 1]) pts.push({ coord: track.coords[k], sample: i })
  })
  return pts
}

/** One color per track point for the chosen mode; null for plain mode / no data. */
export function pointColors(
  tel: Telemetry,
  track: { idx: number[]; coords: LngLat[] },
  mode: ColorMode,
): string[] | null {
  const values = series(tel, mode)
  if (!values) return null
  const known = track.idx.map((i) => values[i]).filter((v): v is number => v !== null && Number.isFinite(v))
  if (!known.length) return null
  const min = mode === 'battery' ? Math.min(0, ...known) : Math.min(...known)
  const max = mode === 'battery' ? 100 : Math.max(...known)
  const ramp = rampFor(mode)
  return track.idx.map((i) => {
    const v = values[i]
    return v === null || !Number.isFinite(v) ? 'rgb(140, 140, 140)' : rampColor(ramp, (v - min) / (max - min || 1))
  })
}
