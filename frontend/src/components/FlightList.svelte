<script lang="ts">
  import { onMount } from 'svelte'
  import { api, type AppConfig, type FlightSummary } from '../lib/api'
  import { formatDateTime, formatDistance, formatDuration, humanizeEnum } from '../lib/format'
  import UploadZone from './UploadZone.svelte'

  let { config }: { config: AppConfig } = $props()

  let flights = $state<FlightSummary[] | null>(null)
  let error = $state<string | null>(null)

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

  onMount(load)
</script>

{#if !config.apiKeyConfigured}
  <div class="alert warn banner">
    <strong>No DJI API key configured.</strong> Logs up to format v{config.encryptedFromVersion - 1} work,
    but encrypted logs from current DJI apps (v{config.encryptedFromVersion}+) cannot be decrypted.
    <p>Set <code>DJI_API_KEY</code> in your <code>.env</code> file and restart the container — see the README.</p>
  </div>
{/if}

<UploadZone {config} onUploaded={load} />

<section>
  <h2>Flights {#if flights}<span class="muted">({flights.length})</span>{/if}</h2>

  {#if error}
    <div class="alert error">{error}</div>
  {:else if flights === null}
    <p class="muted">Loading…</p>
  {:else if flights.length === 0}
    <p class="muted">No flights yet. Upload a log above.</p>
  {:else}
    <div class="table-wrap card">
      <table>
        <thead>
          <tr>
            <th>Date</th>
            <th>Aircraft</th>
            <th class="num">Duration</th>
            <th class="num">Distance</th>
            <th class="num">Max height</th>
            <th>Location</th>
            <th></th>
          </tr>
        </thead>
        <tbody>
          {#each flights as f (f.id)}
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

<style>
  .banner {
    margin-bottom: 1rem;
  }
  section {
    margin-top: 1.5rem;
  }
  h2 {
    font-size: 1.15rem;
    margin: 0 0 0.6rem;
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
