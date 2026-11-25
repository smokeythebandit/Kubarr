import { describe, expect, it } from 'vitest'
import { autoUpdateStatus } from './autoUpdateStatus'
import { emptySchedule, presetSchedule } from '../components/settings/weekSchedule'

// Wednesday 2026-11-25 15:00 UTC.
const now = new Date('2026-11-25T15:00:00Z')
const base = {
  enabled: true,
  schedule: presetSchedule('nightly'),
  timezone: 'UTC',
  health_timeout_minutes: 10,
  next_window_start: '2026-11-26T04:00:00Z',
  timezone_saved: true,
}

describe('autoUpdateStatus', () => {
  it("names tonight's window start in the configured timezone", () => {
    expect(autoUpdateStatus(base, now)).toBe('Auto-update tonight at 04:00 (UTC)')
  })
  it('uses the configured zone, never 24:00, and names the zone', () => {
    const s = { ...base, timezone: 'Europe/Amsterdam', next_window_start: '2026-11-25T23:00:00Z' }
    expect(autoUpdateStatus(s, now)).toBe('Auto-update tonight at 00:00 (Europe/Amsterdam)')
  })
  it('says today for a daytime window later today', () => {
    const s = { ...base, next_window_start: '2026-11-25T16:00:00Z' }
    expect(autoUpdateStatus(s, new Date('2026-11-25T09:00:00Z'))).toBe('Auto-update today at 16:00 (UTC)')
  })
  it('says tomorrow for a daytime window tomorrow', () => {
    const s = { ...base, next_window_start: '2026-11-26T10:00:00Z' }
    expect(autoUpdateStatus(s, now)).toBe('Auto-update tomorrow at 10:00 (UTC)')
  })
  it('names the weekday for a window further away', () => {
    const s = { ...base, schedule: presetSchedule('weekend'), next_window_start: '2026-11-28T01:00:00Z' }
    expect(autoUpdateStatus(s, now)).toBe('Auto-update on Saturday at 01:00 (UTC)')
  })
  it('says the window is open when it already started', () => {
    expect(autoUpdateStatus({ ...base, next_window_start: '2026-11-25T14:00:00Z' }, now)).toBe('Auto-update window open now (UTC)')
  })
  it('says so when every hour is enabled', () => {
    const full = presetSchedule('clear').map((row) => row.map(() => true))
    expect(autoUpdateStatus({ ...base, schedule: full, next_window_start: '2026-11-25T15:00:00Z' }, now)).toBe('Updates can start any time')
  })
  it('says so when no hours are selected', () => {
    expect(autoUpdateStatus({ ...base, schedule: emptySchedule(), next_window_start: null }, now)).toBe('No update hours selected')
  })
  it('says so when automatic updates are off', () => {
    expect(autoUpdateStatus({ ...base, enabled: false }, now)).toBe('Automatic updates are off')
  })
  it('shows nothing while settings are unknown', () => {
    expect(autoUpdateStatus(undefined)).toBeNull()
  })
})
