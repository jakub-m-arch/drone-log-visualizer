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

  let {
    config,
    flight,
    tel,
    cursor,
    follow = false,
  }: { config: AppConfig; flight: FlightSummary; tel: Telemetry; cursor: number; follow?: boolean } = $props()

  let container: HTMLDivElement
  let map: MlMap | null = null
  let drone: Marker | null = null
  let ready = $state(false)

  const track = $derived(trackPoints(tel))

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
