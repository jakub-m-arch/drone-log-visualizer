<script lang="ts">
  import { api, ApiError, errorHint, type AppConfig, type UploadResult } from '../lib/api'

  let { config, onUploaded }: { config: AppConfig; onUploaded: (r: UploadResult) => void } = $props()

  interface Item {
    name: string
    progress: number
    status: 'uploading' | 'parsing' | 'done' | 'duplicate' | 'error'
    error?: ApiError
    flightId?: number
  }

  let items = $state<Item[]>([])
  let dragging = $state(false)
  let input: HTMLInputElement

  async function handleFiles(files: FileList | null) {
    if (!files) return
    // Sequential: decrypting v13+ logs calls the DJI API once per log.
    for (const file of Array.from(files)) {
      items = [{ name: file.name, progress: 0, status: 'uploading' }, ...items]
      // The reactive proxy of the new entry; mutating it updates the UI.
      const it = items[0]
      try {
        const result = await api.upload(file, (p) => {
          it.progress = p
          if (p >= 1) it.status = 'parsing'
        })
        it.status = result.created ? 'done' : 'duplicate'
        it.flightId = result.flight.id
        onUploaded(result)
      } catch (e) {
        it.status = 'error'
        it.error = e instanceof ApiError ? e : new ApiError('unknown', String(e), 0)
      }
    }
  }

  function onDrop(e: DragEvent) {
    e.preventDefault()
    dragging = false
    handleFiles(e.dataTransfer?.files ?? null)
  }
</script>

<div
  class="zone card"
  class:dragging
  role="button"
  tabindex="0"
  aria-label="Upload flight logs"
  ondragover={(e) => {
    e.preventDefault()
    dragging = true
  }}
  ondragleave={() => (dragging = false)}
  ondrop={onDrop}
  onclick={() => input.click()}
  onkeydown={(e) => (e.key === 'Enter' || e.key === ' ') && input.click()}
>
  <input
    bind:this={input}
    type="file"
    accept=".txt"
    multiple
    hidden
    onchange={(e) => {
      handleFiles(e.currentTarget.files)
      e.currentTarget.value = ''
    }}
  />
  <svg width="36" height="36" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" aria-hidden="true">
    <path d="M12 16V4m0 0l-4 4m4-4l4 4M4 16v3a1 1 0 001 1h14a1 1 0 001-1v-3" />
  </svg>
  <strong>Drop <code>DJIFlightRecord_*.txt</code> files here</strong>
  <span class="muted">or click to choose · up to {config.maxUploadMb} MB per file</span>
</div>

{#if items.length}
  <ul class="uploads">
    {#each items as item}
      <li class="card">
        <div class="row">
          <span class="name">{item.name}</span>
          {#if item.status === 'uploading'}
            <span class="muted">Uploading {Math.round(item.progress * 100)}%</span>
          {:else if item.status === 'parsing'}
            <span class="muted">Parsing…</span>
          {:else if item.status === 'done'}
            <a href={`#/flights/${item.flightId}`}>Imported – open</a>
          {:else if item.status === 'duplicate'}
            <a href={`#/flights/${item.flightId}`}>Already imported – open</a>
          {/if}
        </div>
        {#if item.status === 'uploading' || item.status === 'parsing'}
          <progress max="1" value={item.status === 'parsing' ? undefined : item.progress}></progress>
        {/if}
        {#if item.error}
          <div class="alert error">
            {item.error.message}
            {#if errorHint(item.error.code)}<p>{errorHint(item.error.code)}</p>{/if}
          </div>
        {/if}
      </li>
    {/each}
  </ul>
{/if}

<style>
  .zone {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 0.35rem;
    padding: 2rem 1rem;
    border: 2px dashed var(--border);
    cursor: pointer;
    text-align: center;
    color: var(--text-muted);
    transition: border-color 0.15s, background 0.15s;
  }
  .zone strong {
    color: var(--text);
  }
  .zone:hover,
  .zone:focus-visible,
  .zone.dragging {
    border-color: var(--accent);
    background: color-mix(in srgb, var(--accent) 6%, var(--surface));
    outline: none;
  }
  .uploads {
    list-style: none;
    padding: 0;
    margin: 0.75rem 0 0;
    display: grid;
    gap: 0.5rem;
  }
  .uploads li {
    padding: 0.6rem 0.8rem;
    display: grid;
    gap: 0.4rem;
  }
  .row {
    display: flex;
    justify-content: space-between;
    gap: 1rem;
  }
  .name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-family: ui-monospace, monospace;
    font-size: 0.85rem;
  }
  progress {
    width: 100%;
    height: 6px;
  }
</style>
