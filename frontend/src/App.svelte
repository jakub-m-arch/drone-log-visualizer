<script lang="ts">
  import { onMount } from 'svelte'
  import { api, type AppConfig } from './lib/api'
  import FlightList from './components/FlightList.svelte'
  import FlightView from './components/FlightView.svelte'

  let config = $state<AppConfig | null>(null)
  let configError = $state<string | null>(null)
  let route = $state(parseRoute())

  function parseRoute(): { name: 'list' } | { name: 'flight'; id: number } {
    const m = location.hash.match(/^#\/flights\/(\d+)/)
    return m ? { name: 'flight', id: Number(m[1]) } : { name: 'list' }
  }

  onMount(() => {
    const onHash = () => (route = parseRoute())
    window.addEventListener('hashchange', onHash)
    api
      .config()
      .then((c) => (config = c))
      .catch((e) => (configError = e.message))
    return () => window.removeEventListener('hashchange', onHash)
  })
</script>

<header>
  <a class="brand" href="#/">
    <img src="/favicon.svg" alt="" width="24" height="24" />
    Drone Log Visualizer
  </a>
  {#if config}
    <span class="muted small">
      v{config.appVersion} · parser {config.parserVersion} · logs v{config.supportedLogVersions.min}–v{config
        .supportedLogVersions.max}
    </span>
  {/if}
</header>

<main>
  {#if configError}
    <div class="alert error">Cannot reach the backend: {configError}</div>
  {:else if config}
    {#if route.name === 'flight'}
      {#key route.id}
        <FlightView id={route.id} {config} />
      {/key}
    {:else}
      <FlightList {config} />
    {/if}
  {/if}
</main>

<style>
  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 1rem;
    flex-wrap: wrap;
    padding: 0.75rem 1.25rem;
    background: var(--surface);
    border-bottom: 1px solid var(--border);
  }
  .brand {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    font-weight: 650;
    font-size: 1.1rem;
    color: var(--text);
    text-decoration: none;
  }
  .small {
    font-size: 0.8rem;
  }
  main {
    max-width: 1600px;
    margin: 0 auto;
    padding: 1rem 1.25rem 3rem;
  }
</style>
