import { describe, expect, it } from 'vitest'
import type { FlightSummary } from './api'
import { emptyFilter, filterFlights, fleetStats, models } from './stats'
import { isFlightLog, pickFlightLogs } from './files'

function flight(p: Partial<FlightSummary>): FlightSummary {
  return {
    id: 1,
    fileName: 'DJIFlightRecord_a.txt',
    fileSize: 1,
    uploadedAt: '',
    logVersion: 14,
    encrypted: true,
    aircraftName: '',
    aircraftSn: '',
    productType: 'Mini3Pro',
    appPlatform: '',
    appVersion: '',
    startTime: '2024-06-01T10:00:00Z',
    durationS: 600,
    distanceM: 1000,
    maxHeightM: 50,
    maxHSpeedMs: 0,
    maxVSpeedMs: 0,
    homeLat: null,
    homeLon: null,
    takeoffLat: null,
    takeoffLon: null,
    landingLat: null,
    landingLon: null,
    location: 'Warsaw',
    sampleCount: 1,
    ...p,
  }
}

const flights = [
  flight({ id: 1 }),
  flight({ id: 2, productType: 'Mavic3', startTime: '2024-07-15T08:00:00Z', durationS: 1200, maxHeightM: 120, location: 'Kraków' }),
  flight({ id: 3, startTime: null, durationS: 60, distanceM: 10 }),
]

describe('filterFlights', () => {
  it('returns everything for the empty filter', () => {
    expect(filterFlights(flights, emptyFilter)).toHaveLength(3)
  })
  it('filters by model, date range and text', () => {
    expect(filterFlights(flights, { ...emptyFilter, model: 'Mavic3' }).map((f) => f.id)).toEqual([2])
    expect(filterFlights(flights, { ...emptyFilter, from: '2024-07-01' }).map((f) => f.id)).toEqual([2])
    expect(filterFlights(flights, { ...emptyFilter, to: '2024-06-01' }).map((f) => f.id)).toEqual([1])
    expect(filterFlights(flights, { ...emptyFilter, query: 'krak' }).map((f) => f.id)).toEqual([2])
  })
})

describe('fleetStats', () => {
  it('aggregates totals and per-aircraft numbers', () => {
    const s = fleetStats(flights)
    expect(s.flights).toBe(3)
    expect(s.durationS).toBe(1860)
    expect(s.distanceM).toBe(2010)
    expect(s.maxHeightM).toBe(120)
    expect(s.longestFlightS).toBe(1200)
    expect(s.firstFlight).toBe('2024-06-01T10:00:00Z')
    expect(s.lastFlight).toBe('2024-07-15T08:00:00Z')
    expect(s.aircraft).toEqual([
      { productType: 'Mavic3', flights: 1, durationS: 1200, distanceM: 1000 },
      { productType: 'Mini3Pro', flights: 2, durationS: 660, distanceM: 1010 },
    ])
    expect(models(flights)).toEqual(['Mavic3', 'Mini3Pro'])
  })
  it('handles no flights', () => {
    expect(fleetStats([])).toMatchObject({ flights: 0, firstFlight: null, aircraft: [] })
  })
})

describe('flight log files', () => {
  it('recognises DJI log names', () => {
    expect(isFlightLog('DJIFlightRecord_2024-06-01_[10-00-00].txt')).toBe(true)
    expect(isFlightLog('renamed-flight.TXT')).toBe(true)
    expect(isFlightLog('._DJIFlightRecord_x.txt')).toBe(false)
    expect(isFlightLog('FLY001.DAT')).toBe(false)
    expect(isFlightLog('DJIFlightRecord_x.txt.bak')).toBe(false)
  })
  it('picks logs and drops duplicates', () => {
    const f = (name: string, size = 10) => new File([new Uint8Array(size)], name)
    const { logs, skipped } = pickFlightLogs([
      f('DJIFlightRecord_b.txt'),
      f('DJIFlightRecord_a.txt'),
      f('DJIFlightRecord_a.txt'),
      f('.DS_Store'),
    ])
    expect(logs.map((l) => l.name)).toEqual(['DJIFlightRecord_a.txt', 'DJIFlightRecord_b.txt'])
    expect(skipped).toBe(2)
  })
})
