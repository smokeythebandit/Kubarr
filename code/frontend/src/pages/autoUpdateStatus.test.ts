import { afterEach, describe, expect, it, vi } from 'vitest'
import { act, renderHook } from '@testing-library/react'
import { autoUpdateStatus, useAutoUpdateStatus } from './autoUpdateStatus'
import { emptySchedule, presetSchedule } from '../components/settings/weekSchedule'

// Wednesday 2026-11-25 15:00 UTC.
const now = new Date('2026-11-25T15:00:00Z')
const base = {
  enabled: true,
  schedule: presetSchedule('nightly'),
  timezone: 'UTC',
  health_timeout_minutes: 10,
  // Deliberately stale: the line must not depend on it.
  next_window_start: '2026-11-20T04:00:00Z',
  timezone_saved: true,
}

/** Wednesday only, the given local hours. */
function wednesday(...hours: number[]) {
  const s = emptySchedule()
  for (const h of hours) s[2][h] = true
  return s
}

describe('autoUpdateStatus', () => {
  it("names tomorrow's early window as tomorrow", () => {
    expect(autoUpdateStatus(base, now)).toBe('Auto-update tomorrow at 04:00 (UTC)')
  })
  it('uses the configured zone, never 24:00, and names the zone', () => {
    const s = { ...base, timezone: 'Europe/Amsterdam', schedule: wednesday(23) }
    expect(autoUpdateStatus(s, now)).toBe('Auto-update tonight at 23:00 (Europe/Amsterdam)')
    const midnight = emptySchedule()
    midnight[3][0] = true
    expect(autoUpdateStatus({ ...s, schedule: midnight }, now)).toBe('Auto-update tomorrow at 00:00 (Europe/Amsterdam)')
  })
  it('says tonight after midnight for a window later the same local date', () => {
    // 00:30 Amsterdam on Wednesday = Tuesday 23:30 UTC.
    const s = { ...base, timezone: 'Europe/Amsterdam', schedule: wednesday(4) }
    expect(autoUpdateStatus(s, new Date('2026-11-24T23:30:00Z'))).toBe('Auto-update tonight at 04:00 (Europe/Amsterdam)')
  })
  it('says tomorrow after midnight for a window on the following local date', () => {
    const thursday = emptySchedule()
    thursday[3][4] = true
    const s = { ...base, timezone: 'Europe/Amsterdam', schedule: thursday }
    expect(autoUpdateStatus(s, new Date('2026-11-24T23:30:00Z'))).toBe('Auto-update tomorrow at 04:00 (Europe/Amsterdam)')
  })
  it('says today for a daytime window later today', () => {
    expect(autoUpdateStatus({ ...base, schedule: wednesday(16) }, new Date('2026-11-25T09:00:00Z'))).toBe('Auto-update today at 16:00 (UTC)')
  })
  it('names the weekday for a window two or more days away', () => {
    expect(autoUpdateStatus({ ...base, schedule: presetSchedule('weekend') }, now)).toBe('Auto-update on Saturday at 01:00 (UTC)')
    const friday = emptySchedule()
    friday[4][10] = true
    expect(autoUpdateStatus({ ...base, schedule: friday }, now)).toBe('Auto-update on Friday at 10:00 (UTC)')
  })
  it('says the window is open while inside it', () => {
    expect(autoUpdateStatus({ ...base, schedule: wednesday(14, 15) }, now)).toBe('Auto-update window open now (UTC)')
  })
  it('falls back to UTC for an unknown zone', () => {
    expect(autoUpdateStatus({ ...base, timezone: 'Mars/Olympus' }, now)).toBe('Auto-update tomorrow at 04:00 (UTC)')
  })
  it('says so when every hour is enabled', () => {
    const full = presetSchedule('clear').map((row) => row.map(() => true))
    expect(autoUpdateStatus({ ...base, schedule: full }, now)).toBe('Updates can start any time')
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

describe('useAutoUpdateStatus', () => {
  afterEach(() => { vi.useRealTimers() })

  it('stops claiming open once the clock passes the window end', () => {
    vi.useFakeTimers()
    vi.setSystemTime(new Date('2026-11-25T15:58:00Z'))
    const settings = { ...base, schedule: wednesday(14, 15) }
    const { result } = renderHook(() => useAutoUpdateStatus(settings))
    expect(result.current).toBe('Auto-update window open now (UTC)')
    act(() => { vi.advanceTimersByTime(60_000) })
    expect(result.current).toBe('Auto-update window open now (UTC)')
    act(() => { vi.advanceTimersByTime(60_000) })
    expect(result.current).toBe('Auto-update on Wednesday at 14:00 (UTC)')
  })
})
