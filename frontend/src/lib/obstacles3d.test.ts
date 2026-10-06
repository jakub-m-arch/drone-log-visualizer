import { describe, expect, it } from 'vitest'
import type { ObstacleResponse } from './api'
import { distanceM } from './geo'
import { DEFAULT_TREE_HEIGHT_M, lidarColor, lidarExtrusions, treeExtrusions } from './obstacles3d'

const res = (features: ObstacleResponse['data']['features']): ObstacleResponse => ({
  source: 'trees',
  fetchedAt: '',
  cached: false,
  data: { type: 'FeatureCollection', features },
})

describe('treeExtrusions', () => {
  it('uses mapped height and crown, defaults otherwise', () => {
    const t = treeExtrusions(
      res([
        { type: 'Feature', properties: { height: 12, crown: 6 }, geometry: { type: 'Point', coordinates: [21, 52] } },
        { type: 'Feature', properties: { height: null }, geometry: { type: 'Point', coordinates: [21.001, 52] } },
      ]),
    )
    expect(t[0].properties.top).toBe(12)
    expect(t[1].properties.top).toBe(DEFAULT_TREE_HEIGHT_M)
    expect(t[0].properties.color).not.toBe(t[1].properties.color)
    const ring = t[0].geometry.coordinates[0]
    expect(ring).toHaveLength(9)
    expect(ring[0]).toEqual(ring[8])
    // Crown 6 m → radius 3 m.
    expect(distanceM([21, 52], ring[0])).toBeCloseTo(3, 1)
  })
})

describe('lidarExtrusions', () => {
  const block = (height: number, groundRel: number) => ({
    type: 'Feature' as const,
    properties: { height, groundRel },
    geometry: {
      type: 'Polygon' as const,
      coordinates: [
        [
          [21, 52],
          [21.0001, 52],
          [21.0001, 52.0001],
          [21, 52],
        ],
      ] as [number, number][][],
    },
  })
  it('extrudes object height over terrain', () => {
    const [b] = lidarExtrusions(res([block(15, 4)]), true)
    expect(b.properties).toMatchObject({ base: 0, top: 15 })
  })
  it('places blocks at their ground level on flat maps', () => {
    const [b] = lidarExtrusions(res([block(15, 4)]), false)
    expect(b.properties).toMatchObject({ base: 4, top: 19 })
    // Below take-off ground: clipped at 0 and dropped when fully below.
    expect(lidarExtrusions(res([block(3, -5)]), false)).toHaveLength(0)
  })
  it('colors by height', () => {
    expect(lidarColor(2)).toBe('rgb(120, 170, 110)')
    expect(lidarColor(100)).toBe('rgb(120, 80, 60)')
  })
})
