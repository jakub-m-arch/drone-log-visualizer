import { describe, expect, it } from 'vitest'
import { OBJECTS_SOURCE, objectLayers } from './mapObjects'

describe('objectLayers', () => {
  it('defines hidden 3D layers on the OpenMapTiles source', () => {
    const layers = objectLayers()
    expect(layers.map((l) => l.kind)).toEqual(['woods', 'buildings'])
    for (const { layer } of layers) {
      expect(layer.type).toBe('fill-extrusion')
      expect(layer.source).toBe(OBJECTS_SOURCE)
      expect(layer.layout).toEqual({ visibility: 'none' })
    }
    const buildings = layers[1].layer
    expect(buildings['source-layer']).toBe('building')
  })
})
