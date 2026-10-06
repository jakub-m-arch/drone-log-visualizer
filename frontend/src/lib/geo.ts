import type { Telemetry } from './api'

export type LngLat = [number, number]

/** Valid (lng, lat) positions with their sample index. */
export function trackPoints(tel: Telemetry): { idx: number[]; coords: LngLat[] } {
  const idx: number[] = []
  const coords: LngLat[] = []
  for (let i = 0; i < tel.t.length; i++) {
    const lat = tel.lat[i]
    const lon = tel.lon[i]
    if (lat !== null && lon !== null) {
      idx.push(i)
      coords.push([lon, lat])
    }
  }
  return { idx, coords }
}

/** Number of track points at or before sample `cursor` (binary search over `idx`). */
export function pointsUpTo(idx: number[], cursor: number): number {
  let lo = 0
  let hi = idx.length
  while (lo < hi) {
    const mid = (lo + hi) >> 1
    if (idx[mid] <= cursor) lo = mid + 1
    else hi = mid
  }
  return lo
}

export function bounds(coords: LngLat[]): [LngLat, LngLat] | null {
  if (!coords.length) return null
  let [minX, minY] = coords[0]
  let [maxX, maxY] = coords[0]
  for (const [x, y] of coords) {
    if (x < minX) minX = x
    if (x > maxX) maxX = x
    if (y < minY) minY = y
    if (y > maxY) maxY = y
  }
  return [
    [minX, minY],
    [maxX, maxY],
  ]
}

export function distanceM(a: LngLat, b: LngLat): number {
  const R = 6371008.8
  const toRad = Math.PI / 180
  const dLat = (b[1] - a[1]) * toRad
  const dLon = (b[0] - a[0]) * toRad
  const h =
    Math.sin(dLat / 2) ** 2 + Math.cos(a[1] * toRad) * Math.cos(b[1] * toRad) * Math.sin(dLon / 2) ** 2
  return 2 * R * Math.asin(Math.sqrt(h))
}
