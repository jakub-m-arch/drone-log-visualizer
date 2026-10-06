import type { LngLat } from './geo'

/** Ground elevation lookup (metres above sea level), or null when unknown. */
export type GroundAt = (p: LngLat) => number | null

export interface Wall {
  type: 'Feature'
  properties: { base: number; top: number; color: string }
  geometry: { type: 'Polygon'; coordinates: LngLat[][] }
}

/** At most `max` indices from 0..n-1, always including the first and last. */
export function sampleIndices(n: number, max: number): number[] {
  if (n <= 0) return []
  if (n <= max) return Array.from({ length: n }, (_, i) => i)
  const step = (n - 1) / (max - 1)
  const out: number[] = []
  for (let k = 0; k < max; k++) out.push(Math.round(k * step))
  return out
}

const M_PER_DEG_LAT = 111_320

/**
 * Drops points closer than `minM` (horizontal and vertical combined) to the
 * previously kept one, so GPS jitter while hovering does not turn into a
 * pile of tiny, randomly oriented wall segments. Indices refer to `coords`;
 * the first and last points are always kept.
 */
export function thinByDistance(coords: LngLat[], heights: number[], minM: number): number[] {
  if (coords.length <= 2) return coords.map((_, i) => i)
  const keep = [0]
  for (let i = 1; i < coords.length - 1; i++) {
    const j = keep[keep.length - 1]
    const mPerDegLon = M_PER_DEG_LAT * Math.cos(coords[j][1] * (Math.PI / 180))
    const dx = (coords[i][0] - coords[j][0]) * mPerDegLon
    const dy = (coords[i][1] - coords[j][1]) * M_PER_DEG_LAT
    const dz = heights[i] - heights[j]
    if (Math.hypot(dx, dy, dz) >= minM) keep.push(i)
  }
  keep.push(coords.length - 1)
  return keep
}

/** Size of the aircraft box: ~1/30 of the track extent, between 2 and 6 m. */
export function droneSizeM(coords: LngLat[]): number {
  if (coords.length < 2) return 3
  let [minX, minY] = coords[0]
  let [maxX, maxY] = coords[0]
  for (const [x, y] of coords) {
    minX = Math.min(minX, x)
    maxX = Math.max(maxX, x)
    minY = Math.min(minY, y)
    maxY = Math.max(maxY, y)
  }
  const mPerDegLon = M_PER_DEG_LAT * Math.cos(((minY + maxY) / 2) * (Math.PI / 180))
  const extent = Math.hypot((maxX - minX) * mPerDegLon, (maxY - minY) * M_PER_DEG_LAT)
  return Math.min(6, Math.max(2, extent / 30))
}

/** Rectangle of `widthM` around the segment a→b (lng/lat), closed ring. */
export function segmentRect(a: LngLat, b: LngLat, widthM: number): LngLat[] {
  const mPerDegLon = M_PER_DEG_LAT * Math.cos(((a[1] + b[1]) / 2) * (Math.PI / 180))
  const dx = (b[0] - a[0]) * mPerDegLon
  const dy = (b[1] - a[1]) * M_PER_DEG_LAT
  const len = Math.hypot(dx, dy)
  // Perpendicular unit vector; degenerate segments get an arbitrary direction.
  const [px, py] = len > 1e-6 ? [-dy / len, dx / len] : [1, 0]
  const ox = (px * widthM) / 2 / mPerDegLon
  const oy = (py * widthM) / 2 / M_PER_DEG_LAT
  return [
    [a[0] + ox, a[1] + oy],
    [b[0] + ox, b[1] + oy],
    [b[0] - ox, b[1] - oy],
    [a[0] - ox, a[1] - oy],
    [a[0] + ox, a[1] + oy],
  ]
}

function centroid(ring: LngLat[]): LngLat {
  let x = 0
  let y = 0
  const pts = ring.slice(0, -1)
  for (const [px, py] of pts) {
    x += px
    y += py
  }
  return [x / pts.length, y / pts.length]
}

export interface WallOptions {
  /** Height above take-off (m) for each point. */
  heights: number[]
  /** Color for each point. */
  colors: string[]
  widthM: number
  /** Thickness of the floating ribbon; `null` = curtain down to the ground. */
  ribbonM: number | null
  /** Terrain lookup; without it the ground is flat at the take-off level. */
  groundAt?: GroundAt
  /** Ground elevation at take-off, required with `groundAt`. */
  takeoffGround?: number | null
}

/**
 * Builds `fill-extrusion` polygons for a 3D track. MapLibre raises each
 * extrusion by the terrain elevation at the polygon's centroid, so the
 * extrusion height is corrected by (take-off ground − ground at centroid):
 * the top then sits at the true height above the take-off point.
 */
export function buildWalls(coords: LngLat[], o: WallOptions): Wall[] {
  const minH = Math.min(0, ...o.heights)
  const walls: Wall[] = []
  for (let i = 0; i + 1 < coords.length; i++) {
    const ring = segmentRect(coords[i], coords[i + 1], o.widthM)
    let offset = -minH
    if (o.groundAt && o.takeoffGround != null) {
      const g = o.groundAt(centroid(ring))
      if (g != null) offset = o.takeoffGround - g
    }
    const top = Math.max(0, (o.heights[i] + o.heights[i + 1]) / 2 + offset)
    const base = o.ribbonM === null ? 0 : Math.max(0, top - o.ribbonM)
    walls.push({
      type: 'Feature',
      properties: { base: round(base), top: round(top), color: o.colors[i] },
      geometry: { type: 'Polygon', coordinates: [ring] },
    })
  }
  return walls
}

/** A small cube marking the aircraft at `heightM` above take-off. */
export function droneBox(p: LngLat, heightM: number, sizeM: number, groundAt?: GroundAt, takeoffGround?: number | null): Wall {
  let offset = 0
  if (groundAt && takeoffGround != null) {
    const g = groundAt(p)
    if (g != null) offset = takeoffGround - g
  }
  const half = sizeM / 2
  const mPerDegLon = M_PER_DEG_LAT * Math.cos(p[1] * (Math.PI / 180))
  const dx = half / mPerDegLon
  const dy = half / M_PER_DEG_LAT
  const top = Math.max(sizeM, heightM + offset + half)
  return {
    type: 'Feature',
    properties: { base: round(top - sizeM), top: round(top), color: '#111827' },
    geometry: {
      type: 'Polygon',
      coordinates: [
        [
          [p[0] - dx, p[1] - dy],
          [p[0] + dx, p[1] - dy],
          [p[0] + dx, p[1] + dy],
          [p[0] - dx, p[1] + dy],
          [p[0] - dx, p[1] - dy],
        ],
      ],
    },
  }
}

function round(v: number): number {
  return Math.round(v * 100) / 100
}
