import type { FillExtrusionLayerSpecification } from 'maplibre-gl'

/** Source id of the OpenMapTiles-schema vector tiles. */
export const OBJECTS_SOURCE = 'objects'

/** Typical canopy height used for wooded areas, which carry no height data. */
export const WOOD_HEIGHT_M = 15

/** Fallback for buildings without height / levels in OSM. */
export const DEFAULT_BUILDING_HEIGHT_M = 6

export type ObjectKind = 'buildings' | 'woods'

/**
 * 3D layers built from OpenMapTiles vector tiles (as served by OpenFreeMap):
 * - `building.render_height` / `render_min_height` come from OSM `height`,
 *   `min_height` or `building:levels`; parts flagged `hide_3d` are skipped
 *   because a more detailed building:part covers them;
 * - `landcover` with class `wood` (forests and woods) is extruded to an
 *   assumed canopy height.
 */
export function objectLayers(): { kind: ObjectKind; layer: FillExtrusionLayerSpecification }[] {
  return [
    {
      kind: 'woods',
      layer: {
        id: 'woods-3d',
        type: 'fill-extrusion',
        source: OBJECTS_SOURCE,
        'source-layer': 'landcover',
        minzoom: 12,
        filter: ['==', ['get', 'class'], 'wood'],
        layout: { visibility: 'none' },
        paint: {
          'fill-extrusion-color': '#3f7f45',
          'fill-extrusion-height': WOOD_HEIGHT_M,
          'fill-extrusion-base': 0,
          'fill-extrusion-opacity': 0.3,
        },
      },
    },
    {
      kind: 'buildings',
      layer: {
        id: 'buildings-3d',
        type: 'fill-extrusion',
        source: OBJECTS_SOURCE,
        'source-layer': 'building',
        minzoom: 13,
        filter: ['!', ['to-boolean', ['get', 'hide_3d']]],
        layout: { visibility: 'none' },
        paint: {
          'fill-extrusion-color': [
            'interpolate',
            ['linear'],
            ['coalesce', ['get', 'render_height'], DEFAULT_BUILDING_HEIGHT_M],
            0,
            '#d9d4cc',
            30,
            '#b8b0a4',
            100,
            '#958c7f',
          ],
          'fill-extrusion-height': ['coalesce', ['get', 'render_height'], DEFAULT_BUILDING_HEIGHT_M],
          'fill-extrusion-base': ['coalesce', ['get', 'render_min_height'], 0],
          'fill-extrusion-opacity': 0.75,
        },
      },
    },
  ]
}
