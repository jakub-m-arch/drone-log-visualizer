import { describe, expect, it } from 'vitest'
import { formatDistance, formatDuration, humanizeEnum, indexAtTime, formatNumber } from './format'

describe('formatDuration', () => {
  it('formats minutes and hours', () => {
    expect(formatDuration(0)).toBe('0:00')
    expect(formatDuration(65.4)).toBe('1:05')
    expect(formatDuration(3725)).toBe('1:02:05')
    expect(formatDuration(NaN)).toBe('–')
  })
})

describe('formatDistance', () => {
  it('switches to km above 1000 m', () => {
    expect(formatDistance(240.4)).toBe('240 m')
    expect(formatDistance(1234)).toBe('1.23 km')
  })
})

describe('formatNumber', () => {
  it('handles missing values', () => {
    expect(formatNumber(null)).toBe('–')
    expect(formatNumber(12.345, 1, 'm')).toBe('12.3 m')
  })
})

describe('humanizeEnum', () => {
  it('splits DJI product names', () => {
    expect(humanizeEnum('MavicPro')).toBe('Mavic Pro')
    expect(humanizeEnum('MavicAir2S')).toBe('Mavic Air 2S')
    expect(humanizeEnum('Mini3Pro')).toBe('Mini 3 Pro')
    expect(humanizeEnum('None')).toBe('Unknown')
  })
})

describe('indexAtTime', () => {
  const t = [0, 0.1, 0.2, 0.5, 1.0]
  it('finds the last sample at or before x', () => {
    expect(indexAtTime(t, -1)).toBe(0)
    expect(indexAtTime(t, 0.15)).toBe(1)
    expect(indexAtTime(t, 0.5)).toBe(3)
    expect(indexAtTime(t, 0.99)).toBe(3)
    expect(indexAtTime(t, 5)).toBe(4)
    expect(indexAtTime([], 1)).toBe(0)
  })
})
