<script lang="ts">
  import { onMount } from 'svelte'
  import { api, type AppConfig, type FlightSummary } from '../lib/api'
  import { formatDateTime, formatDistance, formatDuration, humanizeEnum } from '../lib/format'
  import { emptyFilter, filterFlights, fleetStats, models, type FlightFilter } from '../lib/stats'
  import UploadZone from './UploadZone.svelte'

  let { config }: { config: AppConfig } = $props()

  let flights = $state<FlightSummary[] | null>(null)
  let error = $state<string | null>(null)
  let filter = $state<FlightFilter>({ ...emptyFilter })

  type SortKey = 'date' | 'duration' | 'distance' | 'height'
  let sortKey = $state<SortKey>('date')
  let sortDesc = $state(true)

  const allModels = $derived(flights ? models(flights) : [])
  const filtered = $derived(flights ? filterFlights(flights, filter) : [])
  const stats = $derived(fleetStats(filtered))
  const isFiltered = $derived(
    filter.model !== '' || filter.from !== '' || filter.to !== '' || filter.query.trim() !== '',
  )

  const sorted = $derived.by(() => {
    const value = (f: FlightSummary): number | string => {
      switch (sortKey) {
        case 'date':
          return f.startTime ?? f.uploadedAt
        case 'duration':
          return f.durationS
        case 'distance':
          return f.distanceM
        case 'height':
          return f.maxHeightM
      }
    }
    const dir = sortDesc ? -1 : 1
    return [...filtered].sort((a, b) => (value(a) < value(b) ? -dir : value(a) > value(b) ? dir : 0))
  })

  function sortBy(key: SortKey) {
    if (sortKey === key) sortDesc = !sortDesc
    else {
      sortKey = key
      sortDesc = true
    }
  }

  function arrow(key: SortKey) {
    return sortKey === key ? (sortDesc ? ' ↓' : ' ↑') : ''
  }

  async function load() {
    try {
      flights = await api.flights()
      error = null
    } catch (e) {
      error = e instanceof Error ? e.message : String(e)
    }
  }

  async function remove(f: FlightSummary) {
    if (!confirm(`Delete flight "${f.fileName}"? This removes it from the database.`)) return
    try {
      await api.deleteFlight(f.id)
      await load()
    } catch (e) {
      error = e instanceof Error ? e.message : String(e)
    }
  }

  function model(f: FlightSummary) {
    return humanizeEnum(f.productType)
  }

  function totalTime(seconds: number) {
    const h = Math.floor(seconds / 3600)
    const m = Math.round((seconds % 3600) / 60)
    return h ? `${h} h ${m} min` : `${m} min`
  }

  function shortDate(iso: string | null) {
    return iso ? new Date(iso).toLocaleDateString(undefined, { year: 'numeric', month: 'short', day: 'numeric' }) : '–'
  }

  onMount(load)
</script>

{#if !config.apiKeyConfigured}
  <div class="alert warn banner">
    <strong>No DJI API key configured.</strong> Logs up to format v{config.encryptedFromVersion - 1} work,
    but encrypted logs from current DJI apps (v{config.encryptedFromVersion}+) cannot be decrypted.
    <p>Set <code>DJI_API_KEY</code> in your <code>.env</code> file and restart the container — see the README.</p>
  </div>
{/if}

<UploadZone {config} onImported={load} />

{#if error}
  <div class="alert error spaced">{error}</div>
{:else if flights === null}
  <p class="muted">Loading…</p>
{:else if flights.length === 0}
  <p class="muted spaced">No flights yet. Upload a log above.</p>
{:else}
  <section class="stats-section">
    <div class="stats">
      <div class="stat card"><span>Flights</span><strong>{stats.flights}</strong></div>
      <div class="stat card"><span>Flight time</span><strong>{totalTime(stats.durationS)}</strong></div>
      <div class="stat card"><span>Distance</span><strong>{formatDistance(stats.distanceM)}</strong></div>
      <div class="stat card"><span>Longest flight</span><strong>{formatDuration(stats.longestFlightS)}</strong></div>
      <div class="stat card"><span>Max height</span><strong>{Math.round(stats.maxHeightM)} m</strong></div>
      <div class="stat card">
        <span>Period</span>
        <strong class="period">{shortDate(stats.firstFlight)} – {shortDate(stats.lastFlight)}</strong>
      </div>
    </div>

    {#if stats.aircraft.length > 1 || (stats.aircraft.length === 1 && !filter.model)}
      <div class="aircraft">
        {#each stats.aircraft as a (a.productType)}
          <button
            class="aircraft-chip"
            class:active={filter.model === a.productType}
            onclick={() => (filter.model = filter.model === a.productType ? '' : a.productType)}
            title="Filter by this aircraft"
          >
            <strong>{humanizeEnum(a.productType)}</strong>
            <span class="muted">{a.flights} flights · {totalTime(a.durationS)} · {formatDistance(a.distanceM)}</span>
          </button>
        {/each}
      </div>
    {/if}
  </section>

  <section>
    <div class="list-head">
      <h2>
        Flights <span class="muted">({isFiltered ? `${filtered.length} of ${flights.length}` : flights.length})</span>
      </h2>
      <div class="filters">
        <input type="search" placeholder="Search name, location…" bind:value={filter.query} aria-label="Search" />
        <select bind:value={filter.model} aria-label="Aircraft">
          <option value="">All aircraft</option>
          {#each allModels as m}<option value={m}>{humanizeEnum(m)}</option>{/each}
        </select>
        <label>From <input type="date" bind:value={filter.from} /></label>
        <label>To <input type="date" bind:value={filter.to} /></label>
        {#if isFiltered}
          <button onclick={() => (filter = { ...emptyFilter })}>Clear</button>
        {/if}
      </div>
    </div>

    {#if filtered.length === 0}
      <p class="muted">No flights match the filters.</p>
    {:else}
      <div class="table-wrap card">
        <table>
          <thead>
            <tr>
              <th><button class="th" onclick={() => sortBy('date')}>Date{arrow('date')}</button></th>
              <th>Aircraft</th>
              <th class="num"><button class="th" onclick={() => sortBy('duration')}>Duration{arrow('duration')}</button></th>
              <th class="num"><button class="th" onclick={() => sortBy('distance')}>Distance{arrow('distance')}</button></th>
              <th class="num"><button class="th" onclick={() => sortBy('height')}>Max height{arrow('height')}</button></th>
              <th>Location</th>
              <th></th>
            </tr>
          </thead>
          <tbody>
            {#each sorted as f (f.id)}
              <tr>
                <td><a href={`#/flights/${f.id}`}>{formatDateTime(f.startTime)}</a></td>
                <td>
                  {model(f)}
                  {#if f.aircraftName && f.aircraftName !== model(f)}
                    <span class="muted">· {f.aircraftName}</span>
                  {/if}
                </td>
                <td class="num">{formatDuration(f.durationS)}</td>
                <td class="num">{formatDistance(f.distanceM)}</td>
                <td class="num">{Math.round(f.maxHeightM)} m</td>
                <td class="muted">{f.location || '–'}</td>
                <td class="actions">
                  <button title="Delete flight" aria-label="Delete flight" onclick={() => remove(f)}>✕</button>
                </td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
    {/if}
  </section>
{/if}

<style>
  .banner {
    margin-bottom: 1rem;
  }
  .spaced {
    margin-top: 1.5rem;
  }
  section {
    margin-top: 1.5rem;
  }
  .stats {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(150px, 1fr));
    gap: 0.6rem;
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
  .stat strong.period {
    font-size: 0.95rem;
  }
  .aircraft {
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem;
    margin-top: 0.6rem;
  }
  .aircraft-chip {
    display: grid;
    text-align: left;
    gap: 0;
    padding: 0.4rem 0.7rem;
    font-size: 0.85rem;
  }
  .aircraft-chip span {
    font-size: 0.78rem;
  }
  .aircraft-chip.active {
    border-color: var(--accent);
    background: color-mix(in srgb, var(--accent) 10%, var(--surface));
  }
  .list-head {
    display: flex;
    justify-content: space-between;
    align-items: center;
    flex-wrap: wrap;
    gap: 0.5rem 1rem;
    margin-bottom: 0.6rem;
  }
  h2 {
    font-size: 1.15rem;
    margin: 0;
  }
  .filters {
    display: flex;
    flex-wrap: wrap;
    gap: 0.4rem;
    align-items: center;
    font-size: 0.85rem;
  }
  .filters input,
  .filters select {
    font: inherit;
    padding: 0.3rem 0.45rem;
    border-radius: 6px;
    border: 1px solid var(--border);
    background: var(--surface);
    color: var(--text);
  }
  .filters label {
    display: flex;
    align-items: center;
    gap: 0.3rem;
    color: var(--text-muted);
  }
  .table-wrap {
    overflow-x: auto;
  }
  table {
    width: 100%;
    border-collapse: collapse;
  }
  th,
  td {
    text-align: left;
    padding: 0.55rem 0.8rem;
    border-bottom: 1px solid var(--border);
    white-space: nowrap;
  }
  th {
    font-size: 0.8rem;
    text-transform: uppercase;
    letter-spacing: 0.03em;
    color: var(--text-muted);
    font-weight: 600;
  }
  button.th {
    border: none;
    background: none;
    padding: 0;
    font: inherit;
    color: inherit;
    text-transform: inherit;
    letter-spacing: inherit;
  }
  tbody tr:last-child td {
    border-bottom: none;
  }
  tbody tr:hover {
    background: var(--surface-2);
  }
  .num {
    text-align: right;
    font-variant-numeric: tabular-nums;
  }
  .actions {
    text-align: right;
    width: 1%;
  }
  .actions button {
    padding: 0.15rem 0.5rem;
    color: var(--text-muted);
  }
</style>
