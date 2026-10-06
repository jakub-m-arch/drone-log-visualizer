import type { FlightSummary } from './api'

export interface FlightFilter {
  model: string // '' = all
  from: string // yyyy-mm-dd, '' = no bound
  to: string
  query: string
}

export const emptyFilter: FlightFilter = { model: '', from: '', to: '', query: '' }

function flightDay(f: FlightSummary): string | null {
  return f.startTime ? f.startTime.slice(0, 10) : null
}

export function filterFlights(flights: FlightSummary[], filter: FlightFilter): FlightSummary[] {
  const q = filter.query.trim().toLowerCase()
  return flights.filter((f) => {
    if (filter.model && f.productType !== filter.model) return false
    const day = flightDay(f)
    if (filter.from && (!day || day < filter.from)) return false
    if (filter.to && (!day || day > filter.to)) return false
    if (q) {
      const hay = `${f.aircraftName} ${f.productType} ${f.location} ${f.fileName}`.toLowerCase()
      if (!hay.includes(q)) return false
    }
    return true
  })
}

export interface AircraftStats {
  productType: string
  flights: number
  durationS: number
  distanceM: number
}

export interface FleetStats {
  flights: number
  durationS: number
  distanceM: number
  maxHeightM: number
  longestFlightS: number
  firstFlight: string | null
  lastFlight: string | null
  aircraft: AircraftStats[]
}

export function fleetStats(flights: FlightSummary[]): FleetStats {
  const byModel = new Map<string, AircraftStats>()
  let durationS = 0
  let distanceM = 0
  let maxHeightM = 0
  let longestFlightS = 0
  const days: string[] = []
  for (const f of flights) {
    durationS += f.durationS
    distanceM += f.distanceM
    maxHeightM = Math.max(maxHeightM, f.maxHeightM)
    longestFlightS = Math.max(longestFlightS, f.durationS)
    if (f.startTime) days.push(f.startTime)
    const a = byModel.get(f.productType) ?? { productType: f.productType, flights: 0, durationS: 0, distanceM: 0 }
    a.flights += 1
    a.durationS += f.durationS
    a.distanceM += f.distanceM
    byModel.set(f.productType, a)
  }
  days.sort()
  return {
    flights: flights.length,
    durationS,
    distanceM,
    maxHeightM,
    longestFlightS,
    firstFlight: days[0] ?? null,
    lastFlight: days[days.length - 1] ?? null,
    aircraft: [...byModel.values()].sort((a, b) => b.durationS - a.durationS),
  }
}

/** Distinct product types, most flown first. */
export function models(flights: FlightSummary[]): string[] {
  return fleetStats(flights).aircraft.map((a) => a.productType)
}
