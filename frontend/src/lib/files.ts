/**
 * DJI app logs are named `DJIFlightRecord_<date>_[<time>].txt`, but renamed
 * logs are fine too, so any visible `.txt` file is a candidate; the server
 * rejects files that are not flight logs.
 */
export function isFlightLog(name: string): boolean {
  return !name.startsWith('.') && /\.txt$/i.test(name)
}

/** Keeps flight logs only; also drops exact duplicates by name + size. */
export function pickFlightLogs(files: File[]): { logs: File[]; skipped: number } {
  const seen = new Set<string>()
  const logs: File[] = []
  for (const f of files) {
    const key = `${f.name}:${f.size}`
    if (!isFlightLog(f.name) || seen.has(key)) continue
    seen.add(key)
    logs.push(f)
  }
  logs.sort((a, b) => a.name.localeCompare(b.name))
  return { logs, skipped: files.length - logs.length }
}

/**
 * Collects all files from a drop, descending into dropped folders.
 * Falls back to `dataTransfer.files` when the entries API is unavailable.
 */
export async function filesFromDrop(dt: DataTransfer): Promise<File[]> {
  const entries = Array.from(dt.items ?? [])
    .map((item) => item.webkitGetAsEntry?.())
    .filter((e): e is FileSystemEntry => !!e)
  if (!entries.length) return Array.from(dt.files)

  const out: File[] = []
  const walk = async (entry: FileSystemEntry): Promise<void> => {
    if (entry.isFile) {
      out.push(await new Promise<File>((res, rej) => (entry as FileSystemFileEntry).file(res, rej)))
    } else if (entry.isDirectory) {
      const reader = (entry as FileSystemDirectoryEntry).createReader()
      // readEntries returns results in batches until it yields an empty array.
      for (;;) {
        const batch = await new Promise<FileSystemEntry[]>((res, rej) => reader.readEntries(res, rej))
        if (!batch.length) break
        for (const child of batch) await walk(child)
      }
    }
  }
  for (const e of entries) await walk(e)
  return out
}
