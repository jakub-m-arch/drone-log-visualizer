import type { ObstacleResponse } from './api'
import type { LngLat } from './geo'
import { rampColor } from './trackColor'

/** Assumed size of a tree when OSM has no `height` / `diameter_crown`. */
export const DEFAULT_TREE_HEIGHT_M = 10
export const DEFAULT_CROWN_M = 5

export interface Extrusion {
  type: 'Feature'
  properties: { base: number; top: number; color: string }
  geometry: { type: 'Polygon'; coordinates: LngLat[][] }
}

const M_PER_DEG_LAT = 111_320

function octagon([lon, lat]: LngLat, radiusM: number): LngLat[] {
  const mPerDegLon = M_PER_DEG_LAT * Math.cos((lat * Math.PI) / 180)
  const ring: LngLat[] = []
  for (let i = 0; i <= 8; i++) {
    const a = ((i % 8) / 8) * 2 * Math.PI
    ring.push([lon + (Math.cos(a) * radiusM) / mPerDegLon, lat + (Math.sin(a) * radiusM) / M_PER_DEG_LAT])
  }
  return ring
}

/**
 * OSM tree points → octagonal columns (crown footprint × height). Trees with
 * mapped heights are darker than ones using the default.
 */
export function treeExtrusions(res: ObstacleResponse): Extrusion[] {
  const out: Extrusion[] = []
  for (const f of res.data.features) {
    if (f.geometry.type !== 'Point') continue
    const height = f.properties.height ?? null
    const crown = f.properties.crown ?? DEFAULT_CROWN_M
    out.push({
      type: 'Feature',
      properties: {
        base: 0,
        top: height ?? DEFAULT_TREE_HEIGHT_M,
        color: height === null ? '#7fb77e' : '#2f7d32',
      },
      geometry: { type: 'Polygon', coordinates: [octagon(f.geometry.coordinates, crown / 2)] },
    })
  }
  return out
}

/**
 * LiDAR blocks → extrusions. With terrain enabled MapLibre already lifts each
 * block by the ground under it, so only the object height is extruded; on flat
 * ground the block is placed at its ground level relative to take-off.
 */
export function lidarExtrusions(res: ObstacleResponse, terrain: boolean): Extrusion[] {
  const out: Extrusion[] = []
  for (const f of res.data.features) {
    if (f.geometry.type !== 'Polygon') continue
    const h = f.properties.height ?? 0
    const g = f.properties.groundRel ?? 0
    const base = terrain ? 0 : Math.max(0, g)
    const top = terrain ? h : Math.max(0, g + h)
    if (top <= base) continue
    out.push({
      type: 'Feature',
      properties: { base, top, color: lidarColor(h) },
      geometry: { type: 'Polygon', coordinates: f.geometry.coordinates as LngLat[][] },
    })
  }
  return out
}

const LIDAR_RAMP: [number, number, number][] = [
  [120, 170, 110],
  [70, 130, 80],
  [150, 120, 80],
  [120, 80, 60],
]

/** Low objects green (vegetation-like) → tall objects brown. */
export function lidarColor(heightM: number): string {
  return rampColor(LIDAR_RAMP, Math.min(1, Math.max(0, (heightM - 2) / 38)))
}
