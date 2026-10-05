export function formatDuration(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds < 0) return '–'
  const s = Math.round(seconds)
  const h = Math.floor(s / 3600)
  const m = Math.floor((s % 3600) / 60)
  const sec = s % 60
  const pad = (n: number) => String(n).padStart(2, '0')
  return h > 0 ? `${h}:${pad(m)}:${pad(sec)}` : `${m}:${pad(sec)}`
}

export function formatDistance(meters: number): string {
  if (!Number.isFinite(meters)) return '–'
  return meters >= 1000 ? `${(meters / 1000).toFixed(2)} km` : `${Math.round(meters)} m`
}

export function formatNumber(value: number | null | undefined, digits = 1, unit = ''): string {
  if (value === null || value === undefined || !Number.isFinite(value)) return '–'
  return `${value.toFixed(digits)}${unit ? ` ${unit}` : ''}`
}

export function formatDateTime(iso: string | null): string {
  if (!iso) return 'Unknown date'
  const d = new Date(iso)
  if (Number.isNaN(d.getTime())) return 'Unknown date'
  return d.toLocaleString(undefined, {
    year: 'numeric',
    month: 'short',
    day: 'numeric',
    hour: '2-digit',
    minute: '2-digit',
  })
}

export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(0)} KB`
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`
}

/** Splits DJI enum names for display: "MavicAir2S" -> "Mavic Air 2S". */
export function humanizeEnum(name: string): string {
  if (!name || name === 'None') return 'Unknown'
  return name
    .replace(/([a-z])([A-Z0-9])/g, '$1 $2')
    .replace(/([0-9])([A-Z][a-z])/g, '$1 $2')
    .replace(/^Unknown\((\d+)\)$/, 'Unknown ($1)')
}

/** Index of the last element of sorted `values` that is <= `x` (0 if none). */
export function indexAtTime(values: ArrayLike<number>, x: number): number {
  let lo = 0
  let hi = values.length - 1
  if (hi < 0) return 0
  if (x <= values[0]) return 0
  if (x >= values[hi]) return hi
  while (lo < hi) {
    const mid = (lo + hi + 1) >> 1
    if (values[mid] <= x) lo = mid
    else hi = mid - 1
  }
  return lo
}
