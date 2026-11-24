import { describe, expect, it } from 'vitest'
import { autoUpdateStatus } from './autoUpdateStatus'

const base = { enabled: true, window_start: '04:00', window_minutes: 120, timezone: 'UTC', health_timeout_minutes: 10, next_window_start: '2026-11-26T04:00:00Z' }

describe('autoUpdateStatus', () => {
  it('names tonight\'s window start in the configured timezone', () => {
    expect(autoUpdateStatus(base)).toBe('Auto-update tonight at 04:00 (UTC)')
  })
  it('uses the configured zone, never 24:00, and names the zone', () => {
    const s = { ...base, timezone: 'Europe/Amsterdam', next_window_start: '2026-11-25T23:00:00Z' }
    expect(autoUpdateStatus(s)).toBe('Auto-update tonight at 00:00 (Europe/Amsterdam)')
  })
  it('says so when automatic updates are off', () => {
    expect(autoUpdateStatus({ ...base, enabled: false })).toBe('Automatic updates are off')
  })
  it('shows nothing while settings are unknown', () => {
    expect(autoUpdateStatus(undefined)).toBeNull()
  })
})
