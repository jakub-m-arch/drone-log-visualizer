import { describe, expect, it } from 'vitest'
import type { Telemetry } from './api'
import { bounds, distanceM, pointsUpTo, trackPoints } from './geo'

const tel = {
  t: [0, 1, 2, 3],
  lat: [52, null, 52.001, 52.002],
  lon: [21, null, 21.001, 21.002],
} as unknown as Telemetry

describe('trackPoints', () => {
  it('skips samples without a fix and keeps indices', () => {
    const { idx, coords } = trackPoints(tel)
    expect(idx).toEqual([0, 2, 3])
    expect(coords[1]).toEqual([21.001, 52.001])
  })
})

describe('pointsUpTo', () => {
  it('counts track points up to a sample index', () => {
    const idx = [0, 2, 3]
    expect(pointsUpTo(idx, 0)).toBe(1)
    expect(pointsUpTo(idx, 1)).toBe(1)
    expect(pointsUpTo(idx, 2)).toBe(2)
    expect(pointsUpTo(idx, 10)).toBe(3)
  })
})

describe('bounds / distance', () => {
  it('computes bounding box', () => {
    expect(bounds(trackPoints(tel).coords)).toEqual([
      [21, 52],
      [21.002, 52.002],
    ])
    expect(bounds([])).toBeNull()
  })
  it('measures ~111 km per degree of latitude', () => {
    expect(Math.round(distanceM([0, 0], [0, 1]) / 1000)).toBe(111)
  })
})
