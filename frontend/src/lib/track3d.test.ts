import { describe, expect, it } from 'vitest'
import { distanceM, type LngLat } from './geo'
import { buildWalls, droneBox, droneSizeM, sampleIndices, segmentRect, thinByDistance } from './track3d'

describe('sampleIndices', () => {
  it('keeps everything under the limit and endpoints over it', () => {
    expect(sampleIndices(3, 10)).toEqual([0, 1, 2])
    const s = sampleIndices(1001, 11)
    expect(s).toHaveLength(11)
    expect(s[0]).toBe(0)
    expect(s[10]).toBe(1000)
    expect(sampleIndices(0, 5)).toEqual([])
  })
})

describe('segmentRect', () => {
  it('builds a closed rectangle of the requested width', () => {
    const a: LngLat = [21, 52]
    const b: LngLat = [21.001, 52]
    const r = segmentRect(a, b, 4)
    expect(r).toHaveLength(5)
    expect(r[0]).toEqual(r[4])
    // Width across the (east-west) segment is ~4 m.
    expect(distanceM(r[0], r[3])).toBeCloseTo(4, 1)
    // Length along it matches the segment.
    expect(distanceM(r[0], r[1])).toBeCloseTo(distanceM(a, b), 3)
  })
})

describe('buildWalls', () => {
  const coords: LngLat[] = [
    [21, 52],
    [21.001, 52],
    [21.002, 52],
  ]
  const colors = ['red', 'green', 'blue']

  it('uses the mean height of each segment on flat ground', () => {
    const w = buildWalls(coords, { heights: [0, 20, 40], colors, widthM: 2, ribbonM: 2 })
    expect(w.map((x) => x.properties.top)).toEqual([10, 30])
    expect(w.map((x) => x.properties.base)).toEqual([8, 28])
    expect(w[1].properties.color).toBe('green')
  })

  it('lifts flights that go below the take-off point', () => {
    const w = buildWalls(coords, { heights: [0, -10, -10], colors, widthM: 2, ribbonM: null })
    expect(w.map((x) => x.properties.top)).toEqual([5, 0])
    expect(w.every((x) => x.properties.base === 0)).toBe(true)
  })

  it('compensates for terrain relative to the take-off ground', () => {
    // Ground rises from 100 m at take-off to 130 m further east.
    const groundAt = (p: LngLat) => 100 + (p[0] - 21) * 20_000
    const w = buildWalls(coords, {
      heights: [50, 50, 50],
      colors,
      widthM: 2,
      ribbonM: null,
      groundAt,
      takeoffGround: 100,
    })
    // Centroids at ground 110 and 130 → extrusion 40 and 20 above local ground,
    // i.e. a constant 150 m above sea level.
    expect(w[0].properties.top).toBeCloseTo(40, 1)
    expect(w[1].properties.top).toBeCloseTo(20, 1)
  })
})

describe('thinByDistance', () => {
  it('drops hover jitter but keeps real movement and the endpoints', () => {
    const d = 0.00001 // ~0.7 m east-west at 52°N
    const coords: LngLat[] = [
      [21, 52],
      [21 + d, 52], // 0.7 m: jitter
      [21, 52], // back
      [21 + 3 * d, 52], // 2 m: kept
      [21 + 3 * d, 52], // same spot but 5 m higher: kept
      [21 + 3 * d, 52], // last: always kept
    ]
    const heights = [0, 0, 0, 0, 5, 5]
    expect(thinByDistance(coords, heights, 1.5)).toEqual([0, 3, 4, 5])
    expect(thinByDistance(coords.slice(0, 2), heights, 1.5)).toEqual([0, 1])
  })
})

describe('droneSizeM', () => {
  it('scales with the track and is clamped', () => {
    expect(droneSizeM([[21, 52], [21.0001, 52]])).toBe(2)
    expect(droneSizeM([[21, 52], [21.002, 52]])).toBeCloseTo(137 / 30, 1)
    expect(droneSizeM([[21, 52], [21.1, 52]])).toBe(6)
  })
})

describe('droneBox', () => {
  it('floats the box at the given height', () => {
    const b = droneBox([21, 52], 30, 4)
    expect(b.properties).toMatchObject({ base: 28, top: 32 })
    expect(b.geometry.coordinates[0]).toHaveLength(5)
  })
})
