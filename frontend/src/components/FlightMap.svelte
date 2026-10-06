<script lang="ts">
  import { onMount } from 'svelte'
  import {
    Map as MlMap,
    Marker,
    NavigationControl,
    ScaleControl,
    setWorkerUrl,
    type GeoJSONSource,
  } from 'maplibre-gl'
  // MapLibre resolves its worker next to its own module, which breaks once
  // bundled; let Vite build the worker (and its imports) as a separate chunk.
  import workerUrl from 'maplibre-gl/dist/maplibre-gl-worker.mjs?worker&url'
  import type { AppConfig, FlightSummary, Telemetry } from '../lib/api'
  import { bounds, pointsUpTo, trackPoints, type LngLat } from '../lib/geo'
  import {
    coloredTrack,
    colorModes,
    photoPoints,
    pointColors,
    rampCss,
    recordingSegments,
    type ColorMode,
  } from '../lib/trackColor'
  import { formatNumber } from '../lib/format'
  import { buildWalls, droneBox, droneSizeM, sampleIndices, thinByDistance, type GroundAt } from '../lib/track3d'

  let {
    config,
    flight,
    tel,
    cursor,
    follow = false,
    onSeek,
  }: {
    config: AppConfig
    flight: FlightSummary
    tel: Telemetry
    cursor: number
    follow?: boolean
    onSeek?: (t: number) => void
  } = $props()

  let container: HTMLDivElement
  let map: MlMap | null = null
  let drone: Marker | null = null
  let ready = $state(false)

  const track = $derived(trackPoints(tel))
  const boxSize = $derived(droneSizeM(track.coords))
  const photos = $derived(photoPoints(tel, track))
  const recordings = $derived(recordingSegments(tel, track))

  const COLOR_KEY = 'trackColorMode'
  let colorMode = $state<ColorMode>(loadColorMode())
  const colored = $derived(coloredTrack(tel, track, colorMode))
  const modeInfo = $derived(colorModes.find((m) => m.value === colorMode)!)

  function loadColorMode(): ColorMode {
    try {
      const v = localStorage.getItem(COLOR_KEY)
      if (colorModes.some((m) => m.value === v)) return v as ColorMode
    } catch {
      /* storage unavailable */
    }
    return 'plain'
  }

  // ---- 3D view ------------------------------------------------------------
  const VIEW_KEY = 'mapView3d'
  let is3d = $state(loadBool(VIEW_KEY))
  const hasTerrain = $derived(!!config.terrainUrl)
  /** Ground elevation at take-off, once terrain tiles are available. */
  let takeoffGround: number | null = null
  let wallsSignature = ''

  function loadBool(key: string): boolean {
    try {
      return localStorage.getItem(key) === '1'
    } catch {
      return false
    }
  }

  function toggle3d() {
    is3d = !is3d
    try {
      localStorage.setItem(VIEW_KEY, is3d ? '1' : '0')
    } catch {
      /* storage unavailable */
    }
  }

  function groundLookup(): GroundAt | undefined {
    if (!map || !is3d || !hasTerrain) return undefined
    const m = map
    return (p) => m.queryTerrainElevation(p)
  }

  /** (Re)builds the 3D ribbon and curtain; skipped when nothing changed. */
  function updateWalls(force = false) {
    if (!map || !ready || !is3d || track.coords.length < 2) return
    const thinned = thinByDistance(
      track.coords,
      track.idx.map((i) => tel.heightM[i] ?? 0),
      1.5,
    )
    const idx = sampleIndices(thinned.length, 1500).map((k) => thinned[k])
    const coords = idx.map((k) => track.coords[k])
    const heights = idx.map((k) => tel.heightM[track.idx[k]] ?? 0)
    const all = pointColors(tel, track, colorMode)
    const colors = idx.map((k) => all?.[k] ?? '#ff6a00')
    const groundAt = groundLookup()
    takeoffGround = groundAt ? groundAt(track.coords[0]) : null
    const grounds = groundAt ? coords.map((c) => Math.round(groundAt(c) ?? -9999)) : []
    const signature = `${colorMode}|${takeoffGround}|${grounds.join(',')}`
    if (!force && signature === wallsSignature) return
    wallsSignature = signature

    const opts = { heights, colors, groundAt, takeoffGround }
    const ribbon = buildWalls(coords, { ...opts, widthM: 1.5, ribbonM: 1 })
    const curtain = buildWalls(coords, { ...opts, widthM: 0.4, ribbonM: null })
    ;(map.getSource('ribbon') as GeoJSONSource).setData({ type: 'FeatureCollection', features: ribbon })
    ;(map.getSource('curtain') as GeoJSONSource).setData({ type: 'FeatureCollection', features: curtain })
    // The take-off ground level may have just become known: re-place the aircraft.
    updateDroneBox()
  }

  function updateDroneBox() {
    if (!map || !ready || !is3d || !track.coords.length) return
    const n = Math.max(1, pointsUpTo(track.idx, cursor))
    const box = droneBox(track.coords[n - 1], tel.heightM[cursor] ?? 0, boxSize, groundLookup(), takeoffGround)
    ;(map.getSource('drone3d') as GeoJSONSource | undefined)?.setData({
      type: 'FeatureCollection',
      features: [box],
    })
  }

  function apply3d() {
    if (!map || !ready) return
    const m = map
    const vis = is3d ? 'visible' : 'none'
    for (const id of ['ribbon', 'curtain', 'drone3d']) m.setLayoutProperty(id, 'visibility', vis)
    if (m.getLayer('hillshade')) m.setLayoutProperty('hillshade', 'visibility', vis)
    if (hasTerrain) m.setTerrain(is3d ? { source: 'dem', exaggeration: 1 } : null)
    // The 2D arrow would sit on the ground; in 3D the floating box replaces it.
    drone?.getElement().style.setProperty('display', is3d ? 'none' : '')
    m.easeTo({ pitch: is3d ? 60 : 0, bearing: is3d ? m.getBearing() : 0, duration: 600 })
    updateWalls(true)
  }

  function setColorMode(mode: ColorMode) {
    colorMode = mode
    try {
      localStorage.setItem(COLOR_KEY, mode)
    } catch {
      /* storage unavailable */
    }
  }

  function markerEl(label: string, cls: string, title: string): HTMLElement {
    const el = document.createElement('div')
    el.className = `pin ${cls}`
    el.textContent = label
    el.title = title
    return el
  }

  function droneEl(): HTMLElement {
    const el = document.createElement('div')
    el.className = 'drone'
    el.innerHTML =
      '<svg viewBox="0 0 24 24" width="30" height="30" aria-hidden="true"><path d="M12 2l7 19-7-4-7 4z" /></svg>'
    return el
  }

  function line(coords: LngLat[]) {
    return {
      type: 'Feature' as const,
      properties: {},
      geometry: { type: 'LineString' as const, coordinates: coords },
    }
  }

  onMount(() => {
    setWorkerUrl(workerUrl)
    const m = new MlMap({
      container,
      style: {
        version: 8,
        sources: {
          osm: {
            type: 'raster',
            tiles: [config.mapTileUrl],
            tileSize: 256,
            maxzoom: 19,
            attribution: config.mapAttribution,
          },
        },
        layers: [{ id: 'osm', type: 'raster', source: 'osm' }],
      },
      center: track.coords[0] ?? [0, 0],
      zoom: track.coords.length ? 15 : 1,
      maxPitch: 80,
      attributionControl: { compact: true },
    })
    map = m
    m.addControl(new NavigationControl({ visualizePitch: true }), 'top-right')
    m.addControl(new ScaleControl({ unit: 'metric' }), 'bottom-left')

    m.on('load', () => {
      if (config.terrainUrl) {
        const dem = {
          type: 'raster-dem' as const,
          tiles: [config.terrainUrl],
          encoding: config.terrainEncoding,
          tileSize: 256,
          maxzoom: 15,
          attribution: config.terrainAttribution,
        }
        // Separate sources for terrain and hillshade, as MapLibre recommends.
        m.addSource('dem', dem)
        m.addSource('dem-hillshade', dem)
        m.addLayer({
          id: 'hillshade',
          type: 'hillshade',
          source: 'dem-hillshade',
          layout: { visibility: 'none' },
          paint: { 'hillshade-exaggeration': 0.4 },
        })
      }
      m.addSource('track', { type: 'geojson', data: line(track.coords) })
      m.addSource('flown', { type: 'geojson', data: line([]) })
      m.addSource('colored', { type: 'geojson', data: { type: 'FeatureCollection', features: [] } })
      m.addSource('recording', {
        type: 'geojson',
        data: {
          type: 'Feature',
          properties: {},
          geometry: { type: 'MultiLineString', coordinates: recordings },
        },
      })
      m.addSource('photos', {
        type: 'geojson',
        data: {
          type: 'FeatureCollection',
          features: photos.map((p) => ({
            type: 'Feature' as const,
            properties: { t: tel.t[p.sample] },
            geometry: { type: 'Point' as const, coordinates: p.coord },
          })),
        },
      })
      // Wide translucent band under the track where video was recorded.
      m.addLayer({
        id: 'recording',
        type: 'line',
        source: 'recording',
        layout: { 'line-join': 'round', 'line-cap': 'round' },
        paint: { 'line-color': '#e11d48', 'line-width': 12, 'line-opacity': 0.3 },
      })
      m.addLayer({
        id: 'track',
        type: 'line',
        source: 'track',
        layout: { 'line-join': 'round', 'line-cap': 'round' },
        paint: { 'line-color': '#ff6a00', 'line-width': 3, 'line-opacity': 0.45 },
      })
      m.addLayer({
        id: 'flown',
        type: 'line',
        source: 'flown',
        layout: { 'line-join': 'round', 'line-cap': 'round' },
        paint: { 'line-color': '#ff6a00', 'line-width': 4 },
      })
      m.addLayer({
        id: 'colored',
        type: 'line',
        source: 'colored',
        layout: { 'line-join': 'round', 'line-cap': 'round' },
        paint: { 'line-color': ['get', 'color'], 'line-width': 4 },
      })
      m.addLayer({
        id: 'photos',
        type: 'circle',
        source: 'photos',
        paint: {
          'circle-radius': 5,
          'circle-color': '#ffffff',
          'circle-stroke-color': '#111827',
          'circle-stroke-width': 2,
        },
      })
      const empty = { type: 'FeatureCollection' as const, features: [] }
      m.addSource('curtain', { type: 'geojson', data: empty })
      m.addSource('ribbon', { type: 'geojson', data: empty })
      m.addSource('drone3d', { type: 'geojson', data: empty })
      const extrusion = (id: string, opacity: number) =>
        m.addLayer({
          id,
          type: 'fill-extrusion',
          source: id,
          layout: { visibility: 'none' },
          paint: {
            'fill-extrusion-color': ['get', 'color'],
            'fill-extrusion-base': ['get', 'base'],
            'fill-extrusion-height': ['get', 'top'],
            'fill-extrusion-opacity': opacity,
          },
        })
      extrusion('curtain', 0.25)
      extrusion('ribbon', 0.95)
      extrusion('drone3d', 1)
      // Terrain tiles arrive asynchronously; refresh the heights once they do.
      m.on('idle', () => updateWalls())

      m.on('click', 'photos', (e) => {
        const t = e.features?.[0]?.properties?.t
        if (typeof t === 'number') onSeek?.(t)
      })
      m.on('mouseenter', 'photos', () => (m.getCanvas().style.cursor = 'pointer'))
      m.on('mouseleave', 'photos', () => (m.getCanvas().style.cursor = ''))

      const add = (lat: number | null, lon: number | null, el: HTMLElement) => {
        if (lat !== null && lon !== null) new Marker({ element: el }).setLngLat([lon, lat]).addTo(m)
      }
      add(flight.homeLat, flight.homeLon, markerEl('H', 'home', 'Home point'))
      add(flight.takeoffLat, flight.takeoffLon, markerEl('T', 'takeoff', 'Take-off'))
      add(flight.landingLat, flight.landingLon, markerEl('L', 'landing', 'Landing'))

      const d = new Marker({ element: droneEl(), rotationAlignment: 'map' })
      if (track.coords.length) d.setLngLat(track.coords[0]).addTo(m)
      drone = d

      const b = bounds(track.coords)
      if (b) m.fitBounds(b, { padding: 50, maxZoom: 17, duration: 0 })
      ready = true
      if (is3d) apply3d()
    })

    const ro = new ResizeObserver(() => m.resize())
    ro.observe(container)
    return () => {
      ro.disconnect()
      m.remove()
      map = null
      drone = null
    }
  })

  // Rebuild the 3D track when the color mode changes.
  $effect(() => {
    void colorMode
    if (ready && is3d) updateWalls()
  })

  // Switch between the plain orange track and the colored one.
  $effect(() => {
    if (!ready || !map) return
    const c = colored
    ;(map.getSource('colored') as GeoJSONSource | undefined)?.setData({
      type: 'FeatureCollection',
      features: c?.features ?? [],
    })
    const plain = c === null
    map.setLayoutProperty('track', 'visibility', plain ? 'visible' : 'none')
    map.setLayoutProperty('flown', 'visibility', plain ? 'visible' : 'none')
  })

  // Move the drone and grow the "flown" line as the cursor moves.
  $effect(() => {
    if (!ready || !map || !drone || !track.coords.length) return
    const n = Math.max(1, pointsUpTo(track.idx, cursor))
    const pos = track.coords[n - 1]
    drone.setLngLat(pos)
    drone.setRotation(tel.yawDeg[cursor] ?? 0)
    if (is3d) updateDroneBox()
    ;(map.getSource('flown') as GeoJSONSource | undefined)?.setData(line(track.coords.slice(0, n)))
    if (follow) map.jumpTo({ center: pos })
  })
</script>

<div class="map" bind:this={container}>
  {#if !track.coords.length}
    <div class="nogps">This flight has no GPS positions.</div>
  {:else}
    <div class="legend">
      <div class="view-toggle" role="group" aria-label="Map view">
        <button class:active={!is3d} onclick={() => is3d && (toggle3d(), apply3d())}>2D</button>
        <button class:active={is3d} onclick={() => !is3d && (toggle3d(), apply3d())}>3D</button>
      </div>
      {#if is3d}
        <div class="muted hint">
          {hasTerrain ? 'Terrain on' : 'Flat ground'} · right-drag / Ctrl+drag to tilt
        </div>
      {/if}
      <label>
        Track
        <select value={colorMode} onchange={(e) => setColorMode(e.currentTarget.value as ColorMode)}>
          {#each colorModes as m}<option value={m.value}>{m.label}</option>{/each}
        </select>
      </label>
      {#if colored}
        <div class="ramp" style:background={rampCss(colorMode)}></div>
        <div class="ramp-labels">
          <span>{formatNumber(colored.min, 0, modeInfo.unit)}</span>
          <span>{formatNumber(colored.max, 0, modeInfo.unit)}</span>
        </div>
      {:else if colorMode !== 'plain'}
        <div class="muted">No {modeInfo.label.toLowerCase()} data</div>
      {/if}
      {#if photos.length || recordings.length}
        <div class="keys">
          {#if photos.length}<span><i class="dot"></i>{photos.length} photo{photos.length > 1 ? 's' : ''}</span>{/if}
          {#if recordings.length}<span><i class="band"></i>video</span>{/if}
        </div>
      {/if}
    </div>
  {/if}
</div>

<style>
  .map {
    position: relative;
    width: 100%;
    height: 100%;
    min-height: 360px;
    border-radius: var(--radius);
    overflow: hidden;
  }
  .nogps {
    position: absolute;
    inset: auto 0 0 0;
    z-index: 2;
    padding: 0.5rem;
    text-align: center;
    background: var(--warn-bg);
    color: var(--warn);
  }
  .legend {
    position: absolute;
    right: 0.6rem;
    bottom: 1.8rem;
    z-index: 2;
    display: grid;
    gap: 0.3rem;
    min-width: 150px;
    padding: 0.45rem 0.6rem;
    font-size: 0.75rem;
    background: color-mix(in srgb, var(--surface) 90%, transparent);
    border: 1px solid var(--border);
    border-radius: 8px;
  }
  .legend label {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.5rem;
    color: var(--text-muted);
  }
  .legend select {
    font: inherit;
    padding: 0.1rem 0.25rem;
    border-radius: 5px;
    border: 1px solid var(--border);
    background: var(--surface);
    color: var(--text);
  }
  .view-toggle {
    display: grid;
    grid-template-columns: 1fr 1fr;
  }
  .view-toggle button {
    justify-content: center;
    padding: 0.15rem 0.4rem;
    font-size: 0.75rem;
    border-radius: 0;
  }
  .view-toggle button:first-child {
    border-radius: 6px 0 0 6px;
  }
  .view-toggle button:last-child {
    border-radius: 0 6px 6px 0;
    border-left: none;
  }
  .view-toggle button.active {
    background: var(--accent);
    border-color: var(--accent);
    color: var(--accent-contrast);
  }
  .hint {
    font-size: 0.7rem;
  }
  .ramp {
    height: 8px;
    border-radius: 4px;
  }
  .ramp-labels,
  .keys {
    display: flex;
    justify-content: space-between;
    gap: 0.6rem;
    font-variant-numeric: tabular-nums;
  }
  .keys span {
    display: flex;
    align-items: center;
    gap: 0.3rem;
  }
  .dot {
    width: 9px;
    height: 9px;
    border-radius: 50%;
    background: #fff;
    border: 2px solid #111827;
  }
  .band {
    width: 16px;
    height: 7px;
    border-radius: 3px;
    background: rgba(225, 29, 72, 0.4);
  }
  .map :global(.pin) {
    width: 24px;
    height: 24px;
    border-radius: 50%;
    display: grid;
    place-items: center;
    font: 700 12px/1 system-ui, sans-serif;
    color: #fff;
    border: 2px solid #fff;
    box-shadow: 0 1px 4px rgba(0, 0, 0, 0.4);
    cursor: default;
  }
  .map :global(.pin.home) {
    background: #1f6feb;
  }
  .map :global(.pin.takeoff) {
    background: #2e7d32;
  }
  .map :global(.pin.landing) {
    background: #c62828;
  }
  .map :global(.drone svg) {
    display: block;
    fill: #111827;
    stroke: #fff;
    stroke-width: 1.5;
    filter: drop-shadow(0 1px 2px rgba(0, 0, 0, 0.5));
  }
</style>
