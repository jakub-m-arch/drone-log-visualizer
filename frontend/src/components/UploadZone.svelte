<script lang="ts">
  import { api, ApiError, errorHint, type AppConfig } from '../lib/api'
  import { filesFromDrop, pickFlightLogs } from '../lib/files'

  let { config, onImported }: { config: AppConfig; onImported: () => void } = $props()

  type Status = 'queued' | 'uploading' | 'parsing' | 'done' | 'updated' | 'duplicate' | 'error'

  interface Item {
    file: File
    status: Status
    progress: number
    error?: ApiError
    flightId?: number
  }

  let items = $state<Item[]>([])
  let skipped = $state(0)
  let running = $state(false)
  let cancelRequested = $state(false)
  let dragging = $state(false)
  let showAll = $state(false)
  let fileInput: HTMLInputElement
  let folderInput: HTMLInputElement

  const count = (s: Status) => items.filter((i) => i.status === s).length
  const finished = $derived(
    items.filter((i) => ['done', 'updated', 'duplicate', 'error'].includes(i.status)).length,
  )
  const current = $derived(items.find((i) => i.status === 'uploading' || i.status === 'parsing'))

  // Failures grouped by error code, so 100 logs without a key show one message.
  const errorGroups = $derived.by(() => {
    const groups = new Map<string, { error: ApiError; files: string[] }>()
    for (const i of items) {
      if (i.status !== 'error' || !i.error) continue
      const g = groups.get(i.error.code) ?? { error: i.error, files: [] }
      g.files.push(i.file.name)
      groups.set(i.error.code, g)
    }
    return [...groups.values()]
  })

  async function enqueue(files: File[]) {
    const { logs, skipped: notLogs } = pickFlightLogs(files)
    if (!running) {
      items = []
      skipped = 0
      showAll = false
    }
    skipped += notLogs
    items.push(...logs.map((file): Item => ({ file, status: 'queued', progress: 0 })))
    if (!running) await run()
  }

  async function run() {
    running = true
    cancelRequested = false
    let sinceRefresh = 0
    // Sequential on purpose: each encrypted log may call the DJI API.
    for (let n = 0; n < items.length; n++) {
      if (cancelRequested) break
      const it = items[n]
      if (it.status !== 'queued') continue
      it.status = 'uploading'
      try {
        const result = await api.upload(it.file, (p) => {
          it.progress = p
          if (p >= 1) it.status = 'parsing'
        })
        it.status = result.created ? 'done' : result.reparsed ? 'updated' : 'duplicate'
        it.flightId = result.flight.id
        if ((result.created || result.reparsed) && ++sinceRefresh >= 10) {
          sinceRefresh = 0
          onImported()
        }
      } catch (e) {
        it.status = 'error'
        it.error = e instanceof ApiError ? e : new ApiError('unknown', String(e), 0)
      }
    }
    running = false
    onImported()
  }

  async function onDrop(e: DragEvent) {
    e.preventDefault()
    dragging = false
    if (e.dataTransfer) await enqueue(await filesFromDrop(e.dataTransfer))
  }

  function onPick(e: Event & { currentTarget: HTMLInputElement }) {
    const files = Array.from(e.currentTarget.files ?? [])
    e.currentTarget.value = ''
    enqueue(files)
  }

  function statusLabel(i: Item) {
    switch (i.status) {
      case 'queued':
        return 'Queued'
      case 'uploading':
        return `Uploading ${Math.round(i.progress * 100)}%`
      case 'parsing':
        return 'Parsing…'
      case 'done':
        return 'Imported'
      case 'updated':
        return 'Updated with new data'
      case 'duplicate':
        return 'Already imported'
      case 'error':
        return i.error?.code ?? 'Error'
    }
  }
</script>

<div
  class="zone card"
  class:dragging
  role="region"
  aria-label="Upload flight logs"
  ondragover={(e) => {
    e.preventDefault()
    dragging = true
  }}
  ondragleave={() => (dragging = false)}
  ondrop={onDrop}
>
  <input bind:this={fileInput} type="file" accept=".txt" multiple hidden onchange={onPick} />
  <input bind:this={folderInput} type="file" webkitdirectory multiple hidden onchange={onPick} />
  <svg width="36" height="36" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" aria-hidden="true">
    <path d="M12 16V4m0 0l-4 4m4-4l4 4M4 16v3a1 1 0 001 1h14a1 1 0 001-1v-3" />
  </svg>
  <strong>Drop flight logs or a whole <code>FlightRecord</code> folder here</strong>
  <div class="buttons">
    <button onclick={() => fileInput.click()}>Choose files</button>
    <button onclick={() => folderInput.click()}>Choose folder</button>
  </div>
  <span class="muted small">Only <code>.txt</code> files are imported · up to {config.maxUploadMb} MB each · duplicates are skipped</span>
</div>

{#if items.length || skipped}
  <div class="batch card" aria-live="polite">
    <div class="summary">
      {#if running}
        <strong>Importing {Math.min(finished + 1, items.length)} / {items.length}</strong>
        {#if current}<span class="muted file">{current.file.name} · {statusLabel(current)}</span>{/if}
      {:else}
        <strong>{cancelRequested ? 'Import cancelled' : 'Import finished'}</strong>
      {/if}
      <span class="chips">
        <span class="chip ok">{count('done')} imported</span>
        {#if count('updated')}<span class="chip ok">{count('updated')} updated</span>{/if}
        {#if count('duplicate')}<span class="chip">{count('duplicate')} already imported</span>{/if}
        {#if count('error')}<span class="chip err">{count('error')} failed</span>{/if}
        {#if skipped}<span class="chip">{skipped} other files skipped</span>{/if}
      </span>
      {#if running}
        <button onclick={() => (cancelRequested = true)} disabled={cancelRequested}>Cancel</button>
      {/if}
    </div>

    {#if items.length}
      <progress max={items.length} value={finished}></progress>
    {/if}

    {#each errorGroups as g (g.error.code)}
      <div class="alert error">
        <strong>{g.files.length} × {g.error.message}</strong>
        {#if errorHint(g.error.code)}<p>{errorHint(g.error.code)}</p>{/if}
        <details>
          <summary>Files</summary>
          <ul class="names">
            {#each g.files as name}<li>{name}</li>{/each}
          </ul>
        </details>
      </div>
    {/each}

    {#if items.length > 1}
      <button class="link" onclick={() => (showAll = !showAll)}>
        {showAll ? 'Hide' : 'Show'} all files
      </button>
    {/if}
    {#if showAll || items.length === 1}
      <ul class="names list">
        {#each items as i}
          <li>
            <span class="file">{i.file.name}</span>
            {#if i.flightId}
              <a href={`#/flights/${i.flightId}`}>{statusLabel(i)} – open</a>
            {:else}
              <span class:errtext={i.status === 'error'} class="muted">{statusLabel(i)}</span>
            {/if}
          </li>
        {/each}
      </ul>
    {/if}
  </div>
{/if}

<style>
  .zone {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 0.45rem;
    padding: 1.6rem 1rem;
    border: 2px dashed var(--border);
    text-align: center;
    color: var(--text-muted);
    transition: border-color 0.15s, background 0.15s;
  }
  .zone strong {
    color: var(--text);
  }
  .zone.dragging {
    border-color: var(--accent);
    background: color-mix(in srgb, var(--accent) 6%, var(--surface));
  }
  .buttons {
    display: flex;
    gap: 0.5rem;
  }
  .small {
    font-size: 0.8rem;
  }
  .batch {
    margin-top: 0.75rem;
    padding: 0.75rem 0.9rem;
    display: grid;
    gap: 0.55rem;
  }
  .summary {
    display: flex;
    align-items: center;
    gap: 0.6rem 1rem;
    flex-wrap: wrap;
  }
  .summary button {
    margin-left: auto;
  }
  .chips {
    display: flex;
    gap: 0.35rem;
    flex-wrap: wrap;
  }
  .chip {
    font-size: 0.78rem;
    padding: 0.1rem 0.5rem;
    border-radius: 999px;
    background: var(--surface-2);
    border: 1px solid var(--border);
  }
  .chip.ok {
    color: var(--ok);
  }
  .chip.err {
    color: var(--error);
  }
  progress {
    width: 100%;
    height: 6px;
  }
  .file {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-family: ui-monospace, monospace;
    font-size: 0.82rem;
    min-width: 0;
  }
  .summary .file {
    max-width: 40ch;
  }
  details summary {
    cursor: pointer;
    color: var(--text-muted);
    margin-top: 0.3rem;
  }
  .names {
    margin: 0.3rem 0 0;
    padding-left: 1.1rem;
    font-family: ui-monospace, monospace;
    font-size: 0.8rem;
    color: var(--text);
    max-height: 200px;
    overflow-y: auto;
  }
  .names.list {
    list-style: none;
    padding: 0;
    max-height: 320px;
  }
  .names.list li {
    display: flex;
    justify-content: space-between;
    gap: 1rem;
    padding: 0.15rem 0;
    border-bottom: 1px solid var(--border);
  }
  .errtext {
    color: var(--error);
  }
  button.link {
    justify-self: start;
    border: none;
    background: none;
    padding: 0;
    color: var(--accent);
    font-size: 0.85rem;
  }
</style>
