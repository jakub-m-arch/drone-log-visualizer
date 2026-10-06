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
    rampCss,
    recordingSegments,
    type ColorMode,
  } from '../lib/trackColor'
  import { formatNumber } from '../lib/format'

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
      attributionControl: { compact: true },
    })
    map = m
    m.addControl(new NavigationControl({ visualizePitch: true }), 'top-right')
    m.addControl(new ScaleControl({ unit: 'metric' }), 'bottom-left')

    m.on('load', () => {
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
    ;(map.getSource('flown') as GeoJSONSource | undefined)?.setData(line(track.coords.slice(0, n)))
    if (follow) map.jumpTo({ center: pos })
  })
</script>

<div class="map" bind:this={container}>
  {#if !track.coords.length}
    <div class="nogps">This flight has no GPS positions.</div>
  {:else}
    <div class="legend">
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
