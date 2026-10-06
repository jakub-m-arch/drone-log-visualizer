<script lang="ts">
  import { onMount } from 'svelte'
  import uPlot from 'uplot'
  import type { Telemetry } from '../lib/api'
  import { formatDuration } from '../lib/format'

  let {
    tel,
    time,
    cursor,
    onSeek,
  }: { tel: Telemetry; time: number; cursor: number; onSeek: (t: number) => void } = $props()

  interface SeriesDef {
    label: string
    values: (number | null)[]
    color: string
    scale?: string
    digits?: number
    stepped?: boolean
  }

  interface ChartDef {
    title: string
    unit: string
    series: SeriesDef[]
    /** Optional second axis on the right. */
    right?: { scale: string; unit: string; range?: [number, number] }
    range?: [number, number]
    /** Hide the chart when every value is 0 (e.g. flights imported before the data existed). */
    hideIfAllZero?: boolean
  }

  const charts: ChartDef[] = $derived([
    {
      title: 'Height',
      unit: 'm',
      series: [{ label: 'Height (m)', values: tel.heightM, color: '#3b82f6' }],
    },
    {
      title: 'Speed',
      unit: 'm/s',
      series: [
        { label: 'Horizontal (m/s)', values: tel.hSpeedMs, color: '#f97316' },
        { label: 'Vertical (m/s)', values: tel.vSpeedMs, color: '#a855f7' },
      ],
    },
    {
      title: 'Battery',
      unit: '%',
      range: [0, 100],
      series: [
        { label: 'Charge (%)', values: tel.batteryPct, color: '#22c55e', digits: 0 },
        { label: 'Voltage (V)', values: tel.batteryV, color: '#eab308', scale: 'v', digits: 2 },
      ],
      right: { scale: 'v', unit: 'V' },
    },
    {
      title: 'GPS satellites',
      unit: '',
      series: [{ label: 'Satellites', values: tel.gpsSats, color: '#14b8a6', digits: 0, stepped: true }],
    },
    {
      title: 'Gimbal pitch',
      unit: '°',
      range: [-90, 30],
      hideIfAllZero: true,
      series: [{ label: 'Pitch (°)', values: tel.gimbalPitchDeg ?? [], color: '#0ea5e9' }],
    },
    {
      title: 'RC signal',
      unit: '%',
      range: [0, 100],
      series: [
        { label: 'Uplink (%)', values: tel.rcUplinkPct, color: '#ef4444', digits: 0 },
        { label: 'Downlink (%)', values: tel.rcDownlinkPct, color: '#6366f1', digits: 0 },
      ],
    },
  ])

  let container: HTMLDivElement
  let plots: uPlot[] = []
  let playheads: HTMLDivElement[] = []
  let syncingScale = false

  function cssVar(name: string, fallback: string) {
    return getComputedStyle(document.documentElement).getPropertyValue(name).trim() || fallback
  }

  function hasData(values: (number | null)[]) {
    return values.some((v) => v !== null)
  }

  function placePlayhead(u: uPlot, el: HTMLDivElement) {
    const left = u.valToPos(time, 'x')
    const visible = Number.isFinite(left) && left >= 0 && left <= u.over.clientWidth
    el.style.display = visible ? 'block' : 'none'
    if (visible) el.style.transform = `translateX(${left}px)`
  }

  function build() {
    plots.forEach((p) => p.destroy())
    plots = []
    playheads = []
    container.replaceChildren()

    const width = container.clientWidth
    const axisColor = cssVar('--text-muted', '#666')
    const grid = { stroke: cssVar('--grid', 'rgba(0,0,0,.08)'), width: 1 }
    const sync = uPlot.sync('flight')

    for (const def of charts) {
      const series = def.series.filter((s) => hasData(s.values))
      if (!series.length) continue
      if (def.hideIfAllZero && series.every((s) => s.values.every((v) => !v))) continue

      const wrap = document.createElement('div')
      wrap.className = 'chart'
      container.appendChild(wrap)

      const scales: uPlot.Scales = {
        x: { time: false },
        y: def.range ? { range: def.range } : { auto: true },
      }
      const axes: uPlot.Axis[] = [
        {
          stroke: axisColor,
          grid,
          ticks: grid,
          values: (_u, splits) => splits.map((s) => formatDuration(s)),
        },
        { stroke: axisColor, grid, ticks: grid, label: def.unit, size: 50, labelSize: 14 },
      ]
      if (def.right && series.some((s) => s.scale === def.right!.scale)) {
        scales[def.right.scale] = def.right.range ? { range: def.right.range } : { auto: true }
        axes.push({
          scale: def.right.scale,
          side: 1,
          stroke: axisColor,
          grid: { show: false },
          ticks: grid,
          label: def.right.unit,
          size: 50,
          labelSize: 14,
        })
      }

      const opts: uPlot.Options = {
        title: def.title,
        width,
        height: 170,
        scales,
        axes,
        cursor: {
          sync: { key: sync.key, setSeries: false },
          drag: { x: true, y: false, setScale: true },
        },
        legend: { live: true },
        series: [
          { label: 'Time', value: (_u, v) => (v == null ? '–' : formatDuration(v)) },
          ...series.map(
            (s): uPlot.Series => ({
              label: s.label,
              stroke: s.color,
              width: 1.5,
              scale: s.scale ?? 'y',
              spanGaps: false,
              points: { show: false },
              paths: s.stepped ? uPlot.paths.stepped!({ align: 1 }) : undefined,
              value: (_u, v) => (v == null ? '–' : v.toFixed(s.digits ?? 1)),
            }),
          ),
        ],
        hooks: {
          setScale: [
            (u, key) => {
              if (key !== 'x' || syncingScale) return
              // Keep the zoom window identical across all charts.
              syncingScale = true
              const { min, max } = u.scales.x
              for (const other of plots) {
                if (other !== u && min != null && max != null) other.setScale('x', { min, max })
              }
              syncingScale = false
              plots.forEach((p, i) => placePlayhead(p, playheads[i]))
            },
          ],
          setSize: [(u) => placePlayhead(u, playheads[plots.indexOf(u)])],
        },
      }

      const data: uPlot.AlignedData = [tel.t, ...series.map((s) => s.values as (number | null)[])]
      const u = new uPlot(opts, data, wrap)
      sync.sub(u)

      const head = document.createElement('div')
      head.className = 'playhead'
      u.over.appendChild(head)

      // A click (not a drag-to-zoom) seeks to that time.
      let downX = 0
      u.over.addEventListener('mousedown', (e) => (downX = e.clientX))
      u.over.addEventListener('click', (e) => {
        if (Math.abs(e.clientX - downX) > 3) return
        const left = u.cursor.left
        if (left != null && left >= 0) onSeek(u.posToVal(left, 'x'))
      })

      plots.push(u)
      playheads.push(head)
      placePlayhead(u, head)
    }
  }

  function resetZoom() {
    const t = tel.t
    if (!t.length) return
    for (const p of plots) p.setScale('x', { min: t[0], max: t[t.length - 1] })
  }

  onMount(() => {
    build()
    let lastWidth = container.clientWidth
    const ro = new ResizeObserver(() => {
      const w = container.clientWidth
      if (w === lastWidth) return
      lastWidth = w
      for (const p of plots) p.setSize({ width: w, height: 170 })
    })
    ro.observe(container)
    return () => {
      ro.disconnect()
      plots.forEach((p) => p.destroy())
    }
  })

  $effect(() => {
    // Track `time` and keep playheads in place.
    void time
    const idx = cursor
    plots.forEach((p, i) => {
      placePlayhead(p, playheads[i])
      // While the mouse is not over a chart, the legend shows playhead values.
      if (p.cursor.left == null || p.cursor.left < 0) p.setLegend({ idx })
    })
  })
</script>

<div class="toolbar">
  <span class="muted">Drag on a chart to zoom, click to jump to a moment.</span>
  <button onclick={resetZoom}>Reset zoom</button>
</div>
<div class="charts" bind:this={container}></div>

<style>
  .toolbar {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 0.5rem;
    font-size: 0.8rem;
    margin-bottom: 0.25rem;
  }
  .toolbar button {
    padding: 0.15rem 0.6rem;
    font-size: 0.8rem;
  }
  .charts {
    display: grid;
    gap: 0.25rem;
    min-width: 0;
  }
  .charts :global(.chart) {
    min-width: 0;
  }
  .charts :global(.u-title) {
    font-size: 0.85rem;
    font-weight: 600;
    text-align: left;
    padding-left: 0.5rem;
  }
  .charts :global(.u-legend) {
    font-size: 0.75rem;
    color: var(--text);
  }
  .charts :global(.u-legend .u-marker) {
    width: 0.8em;
    height: 0.8em;
  }
  .charts :global(.u-over) {
    cursor: crosshair;
  }
  .charts :global(.u-select) {
    background: color-mix(in srgb, var(--accent) 18%, transparent);
  }
  .charts :global(.playhead) {
    position: absolute;
    top: 0;
    bottom: 0;
    left: 0;
    width: 2px;
    margin-left: -1px;
    background: var(--track);
    pointer-events: none;
  }
</style>
