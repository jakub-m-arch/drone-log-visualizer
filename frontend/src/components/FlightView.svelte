<script lang="ts">
  import { onMount } from 'svelte'
  import { api, type AppConfig, type FlightDetail, type Telemetry } from '../lib/api'
  import {
    formatDateTime,
    formatDistance,
    formatDuration,
    formatNumber,
    humanizeEnum,
    indexAtTime,
  } from '../lib/format'
  import { distanceM } from '../lib/geo'
  import Charts from './Charts.svelte'
  import FlightMap from './FlightMap.svelte'

  let { id, config }: { id: number; config: AppConfig } = $props()

  let detail = $state<FlightDetail | null>(null)
  let tel = $state.raw<Telemetry | null>(null)
  let error = $state<string | null>(null)

  /** Playback position in seconds since the first sample. */
  let time = $state(0)
  let playing = $state(false)
  let speed = $state(5)
  let follow = $state(false)

  const speeds = [1, 2, 5, 10, 20, 50]
  const duration = $derived(tel && tel.t.length ? tel.t[tel.t.length - 1] : 0)
  const cursor = $derived(tel ? indexAtTime(tel.t, time) : 0)

  const hud = $derived.by(() => {
    if (!tel || !detail || !tel.t.length) return null
    const i = cursor
    const f = detail.flight
    const lat = tel.lat[i]
    const lon = tel.lon[i]
    const fromHome =
      lat !== null && lon !== null && f.homeLat !== null && f.homeLon !== null
        ? distanceM([f.homeLon, f.homeLat], [lon, lat])
        : null
    return {
      clock: tel.timestampMs[i] ? new Date(tel.timestampMs[i]!).toLocaleTimeString() : null,
      height: tel.heightM[i],
      hSpeed: tel.hSpeedMs[i],
      vSpeed: tel.vSpeedMs[i],
      battery: tel.batteryPct[i],
      voltage: tel.batteryV[i],
      sats: tel.gpsSats[i],
      rc: tel.rcDownlinkPct[i] ?? tel.rcUplinkPct[i],
      mode: tel.flightMode[i],
      fromHome,
    }
  })

  // Most recent event at or before the playhead.
  const activeEvent = $derived.by(() => {
    if (!detail) return -1
    let idx = -1
    detail.events.forEach((e, i) => {
      if (e.t <= time) idx = i
    })
    return idx
  })

  onMount(() => {
    Promise.all([api.flight(id), api.telemetry(id)])
      .then(([d, t]) => {
        detail = d
        tel = t
      })
      .catch((e) => (error = e.message))

    let raf = 0
    let last = performance.now()
    const tick = (now: number) => {
      const dt = (now - last) / 1000
      last = now
      if (playing) {
        time = Math.min(duration, time + dt * speed)
        if (time >= duration) playing = false
      }
      raf = requestAnimationFrame(tick)
    }
    raf = requestAnimationFrame(tick)

    const onKey = (e: KeyboardEvent) => {
      if (e.target instanceof HTMLInputElement && e.target.type !== 'range') return
      if (e.code === 'Space') {
        e.preventDefault()
        togglePlay()
      } else if (e.code === 'ArrowRight') {
        seek(time + (e.shiftKey ? 30 : 5))
      } else if (e.code === 'ArrowLeft') {
        seek(time - (e.shiftKey ? 30 : 5))
      }
    }
    window.addEventListener('keydown', onKey)
    return () => {
      cancelAnimationFrame(raf)
      window.removeEventListener('keydown', onKey)
    }
  })

  function seek(t: number) {
    time = Math.max(0, Math.min(duration, t))
  }

  function togglePlay() {
    if (!playing && time >= duration) time = 0
    playing = !playing
  }

  async function remove() {
    if (!detail || !confirm(`Delete flight "${detail.flight.fileName}"?`)) return
    await api.deleteFlight(id)
    location.hash = '#/'
  }
</script>

<a href="#/" class="back">← All flights</a>

{#if error}
  <div class="alert error">{error}</div>
{:else if !detail || !tel}
  <p class="muted">Loading flight…</p>
{:else}
  {@const f = detail.flight}
  <div class="head">
    <div>
      <h1>{humanizeEnum(f.productType)} <span class="muted">{f.aircraftName}</span></h1>
      <div class="muted">
        {formatDateTime(f.startTime)}{f.location ? ` · ${f.location}` : ''} · log v{f.logVersion}{f.encrypted
          ? ' (encrypted)'
          : ''} · {f.fileName}
      </div>
    </div>
    <div class="exports">
      <a class="button" href={api.exportUrl(id, 'csv')} download>CSV</a>
      <a class="button" href={api.exportUrl(id, 'gpx')} download>GPX</a>
      <a class="button" href={api.exportUrl(id, 'kml')} download>KML</a>
      <button onclick={remove} title="Delete flight">Delete</button>
    </div>
  </div>

  <div class="stats">
    <div class="stat card"><span>Duration</span><strong>{formatDuration(f.durationS)}</strong></div>
    <div class="stat card"><span>Distance</span><strong>{formatDistance(f.distanceM)}</strong></div>
    <div class="stat card"><span>Max height</span><strong>{formatNumber(f.maxHeightM, 0, 'm')}</strong></div>
    <div class="stat card">
      <span>Max speed</span><strong>{formatNumber(f.maxHSpeedMs, 1, 'm/s')}</strong>
    </div>
    <div class="stat card">
      <span>Max vertical</span><strong>{formatNumber(f.maxVSpeedMs, 1, 'm/s')}</strong>
    </div>
    <div class="stat card"><span>Samples</span><strong>{f.sampleCount.toLocaleString()}</strong></div>
  </div>

  <div class="layout">
    <div class="left">
      <div class="map-wrap card">
        <FlightMap {config} flight={f} {tel} {cursor} {follow} />
        {#if hud}
          <div class="hud">
            <div><span>Height</span>{formatNumber(hud.height, 1, 'm')}</div>
            <div><span>H speed</span>{formatNumber(hud.hSpeed, 1, 'm/s')}</div>
            <div><span>V speed</span>{formatNumber(hud.vSpeed, 1, 'm/s')}</div>
            <div><span>From home</span>{hud.fromHome === null ? '–' : formatDistance(hud.fromHome)}</div>
            <div><span>Battery</span>{formatNumber(hud.battery, 0, '%')} · {formatNumber(hud.voltage, 2, 'V')}</div>
            <div><span>GPS</span>{hud.sats} sats</div>
            <div><span>Signal</span>{formatNumber(hud.rc, 0, '%')}</div>
            <div><span>Mode</span>{humanizeEnum(hud.mode)}</div>
          </div>
        {/if}
      </div>

      <div class="timeline card">
        <button class="primary play" onclick={togglePlay} aria-label={playing ? 'Pause' : 'Play'}>
          {playing ? '❚❚' : '▶'}
        </button>
        <input
          type="range"
          min="0"
          max={duration}
          step="0.1"
          value={time}
          oninput={(e) => seek(Number(e.currentTarget.value))}
          aria-label="Flight time"
        />
        <span class="time">
          {formatDuration(time)} / {formatDuration(duration)}
          {#if hud?.clock}<span class="muted">· {hud.clock}</span>{/if}
        </span>
        <label>
          <select bind:value={speed} aria-label="Playback speed">
            {#each speeds as s}<option value={s}>{s}×</option>{/each}
          </select>
        </label>
        <label class="follow"><input type="checkbox" bind:checked={follow} /> Follow</label>
      </div>

      <div class="events card">
        <h2>Events & warnings <span class="muted">({detail.events.length})</span></h2>
        {#if detail.events.length === 0}
          <p class="muted">No events recorded in this log.</p>
        {:else}
          <ul>
            {#each detail.events as e, i}
              <li class:warning={e.level === 'warning'} class:active={i === activeEvent} class:future={e.t > time}>
                <button onclick={() => seek(e.t)}>
                  <span class="t">{formatDuration(e.t)}</span>
                  <span class="icon" aria-hidden="true">{e.level === 'warning' ? '⚠' : 'ℹ'}</span>
                  <span class="msg">{e.message}</span>
                </button>
              </li>
            {/each}
          </ul>
        {/if}
      </div>
    </div>

    <div class="right card">
      <Charts {tel} {time} onSeek={seek} />
    </div>
  </div>
  <p class="muted keys">Keyboard: Space play/pause · ←/→ ±5 s · Shift+←/→ ±30 s</p>
{/if}

<style>
  .back {
    display: inline-block;
    margin-bottom: 0.75rem;
    text-decoration: none;
  }
  .head {
    display: flex;
    justify-content: space-between;
    align-items: flex-start;
    gap: 1rem;
    flex-wrap: wrap;
  }
  h1 {
    margin: 0;
    font-size: 1.4rem;
  }
  h1 .muted {
    font-weight: 400;
    font-size: 1rem;
  }
  .exports {
    display: flex;
    gap: 0.4rem;
    flex-wrap: wrap;
  }
  .stats {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(130px, 1fr));
    gap: 0.6rem;
    margin: 1rem 0;
  }
  .stat {
    padding: 0.55rem 0.8rem;
    display: grid;
  }
  .stat span {
    font-size: 0.75rem;
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: 0.03em;
  }
  .stat strong {
    font-size: 1.15rem;
    font-variant-numeric: tabular-nums;
  }
  .layout {
    display: grid;
    grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
    gap: 1rem;
    align-items: start;
  }
  @media (max-width: 1000px) {
    .layout {
      grid-template-columns: minmax(0, 1fr);
    }
  }
  .left {
    display: grid;
    gap: 0.75rem;
    min-width: 0;
  }
  .map-wrap {
    position: relative;
    height: 480px;
    padding: 0;
  }
  .hud {
    position: absolute;
    left: 0.6rem;
    top: 0.6rem;
    z-index: 2;
    display: grid;
    grid-template-columns: auto auto;
    gap: 0.1rem 0.9rem;
    padding: 0.45rem 0.65rem;
    font-size: 0.78rem;
    font-variant-numeric: tabular-nums;
    background: color-mix(in srgb, var(--surface) 88%, transparent);
    border: 1px solid var(--border);
    border-radius: 8px;
    pointer-events: none;
  }
  .hud span {
    display: block;
    font-size: 0.65rem;
    color: var(--text-muted);
    text-transform: uppercase;
  }
  .timeline {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    padding: 0.5rem 0.75rem;
    flex-wrap: wrap;
  }
  .timeline input[type='range'] {
    flex: 1 1 200px;
    accent-color: var(--track);
  }
  .play {
    width: 2.4rem;
    justify-content: center;
  }
  .time {
    font-variant-numeric: tabular-nums;
    font-size: 0.9rem;
    white-space: nowrap;
  }
  select {
    font: inherit;
    padding: 0.25rem;
    border-radius: 6px;
    border: 1px solid var(--border);
    background: var(--surface);
    color: var(--text);
  }
  .follow {
    font-size: 0.9rem;
    display: flex;
    align-items: center;
    gap: 0.25rem;
  }
  .events {
    padding: 0.6rem 0.75rem;
  }
  .events h2 {
    font-size: 1rem;
    margin: 0 0 0.4rem;
  }
  .events ul {
    list-style: none;
    margin: 0;
    padding: 0;
    max-height: 300px;
    overflow-y: auto;
  }
  .events li button {
    width: 100%;
    border: none;
    border-radius: 6px;
    background: transparent;
    padding: 0.3rem 0.4rem;
    text-align: left;
    display: grid;
    grid-template-columns: 3.5rem 1.2rem 1fr;
    gap: 0.3rem;
  }
  .events li button:hover {
    background: var(--surface-2);
  }
  .events li.warning .icon,
  .events li.warning .msg {
    color: var(--warn);
  }
  .events li.active button {
    background: color-mix(in srgb, var(--track) 15%, transparent);
  }
  .events li.future {
    opacity: 0.6;
  }
  .events .t {
    font-variant-numeric: tabular-nums;
    color: var(--text-muted);
  }
  .right {
    padding: 0.6rem 0.75rem;
    min-width: 0;
  }
  .keys {
    font-size: 0.8rem;
    margin-top: 1rem;
  }
</style>
