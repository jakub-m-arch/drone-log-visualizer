import { describe, expect, it } from 'vitest'
import type { Telemetry } from './api'
import { trackPoints } from './geo'
import { coloredTrack, photoPoints, rampColor, recordingSegments } from './trackColor'

function tel(n: number, over: Partial<Telemetry> = {}): Telemetry {
  const arr = <T>(f: (i: number) => T) => Array.from({ length: n }, (_, i) => f(i))
  return {
    t: arr((i) => i),
    lat: arr(() => 52),
    lon: arr((i) => 21 + i * 0.0001),
    heightM: arr((i) => i * 10),
    hSpeedMs: arr(() => 5),
    batteryPct: arr(() => null),
    isPhoto: arr(() => false),
    isRecording: arr(() => false),
    ...over,
  } as unknown as Telemetry
}

describe('rampColor', () => {
  const ramp: [number, number, number][] = [
    [0, 0, 0],
    [100, 200, 250],
  ]
  it('interpolates and clamps', () => {
    expect(rampColor(ramp, 0)).toBe('rgb(0, 0, 0)')
    expect(rampColor(ramp, 0.5)).toBe('rgb(50, 100, 125)')
    expect(rampColor(ramp, 2)).toBe('rgb(100, 200, 250)')
    expect(rampColor(ramp, NaN)).toBe('rgb(0, 0, 0)')
  })
})

describe('coloredTrack', () => {
  it('builds contiguous segments with min/max of the series', () => {
    const t = tel(11)
    const track = trackPoints(t)
    const c = coloredTrack(t, track, 'height', 5)!
    expect(c.min).toBe(0)
    expect(c.max).toBe(100)
    expect(c.features).toHaveLength(5)
    // Each segment starts where the previous one ended.
    for (let i = 1; i < c.features.length; i++) {
      const prev = c.features[i - 1].geometry.coordinates
      expect(c.features[i].geometry.coordinates[0]).toEqual(prev[prev.length - 1])
    }
    expect(c.features[0].properties.color).not.toBe(c.features[4].properties.color)
  })
  it('returns null for plain mode or missing data', () => {
    const t = tel(5)
    expect(coloredTrack(t, trackPoints(t), 'plain')).toBeNull()
    expect(coloredTrack(t, trackPoints(t), 'battery')).toBeNull()
  })
})

describe('camera overlays', () => {
  it('finds recording runs and photo starts', () => {
    const t = tel(8, {
      isRecording: [false, true, true, true, false, true, false, false],
      isPhoto: [false, false, true, true, false, false, false, true],
    })
    const track = trackPoints(t)
    // The single-sample run at index 5 cannot form a line and is dropped.
    expect(recordingSegments(t, track).map((s) => s.length)).toEqual([3])
    expect(photoPoints(t, track).map((p) => p.sample)).toEqual([2, 7])
  })
})
