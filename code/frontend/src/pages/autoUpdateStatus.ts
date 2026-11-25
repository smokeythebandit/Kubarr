import { useEffect, useState } from 'react'
import type { UpdateSettingsResponse } from '../api/updates'
import { formatHour, nowInZone, validZone, windowPosition } from '../components/settings/weekSchedule'

const WEEKDAYS = ['Monday', 'Tuesday', 'Wednesday', 'Thursday', 'Friday', 'Saturday', 'Sunday']

/**
 * The card's auto-update status line; null when the viewer cannot see the
 * settings. Computed from the schedule and zone against `now`, so it stays
 * right between refetches.
 */
export function autoUpdateStatus(settings: UpdateSettingsResponse | undefined, now: Date = new Date()): string | null {
  if (!settings) return null
  if (!settings.enabled) return 'Automatic updates are off'
  const zone = validZone(settings.timezone)
  const pos = windowPosition(settings.schedule ?? [], nowInZone(zone, now))
  if (pos.kind === 'empty') return 'No update hours selected'
  if (pos.kind === 'always') return 'Updates can start any time'
  if (pos.kind === 'open') return `Auto-update window open now (${zone})`

  // Same local date: tonight or today by the hour; next date: tomorrow.
  let when: string
  if (pos.days === 0) when = pos.hour >= 18 || pos.hour < 6 ? 'tonight' : 'today'
  else if (pos.days === 1) when = 'tomorrow'
  else when = `on ${WEEKDAYS[pos.day]}`
  return `Auto-update ${when} at ${formatHour(pos.hour)} (${zone})`
}

/** The current time, refreshed every `intervalMs` while mounted. */
export function useNow(intervalMs = 60_000): Date {
  const [now, setNow] = useState(() => new Date())
  useEffect(() => {
    const id = window.setInterval(() => setNow(new Date()), intervalMs)
    return () => window.clearInterval(id)
  }, [intervalMs])
  return now
}

/** {@link autoUpdateStatus} re-evaluated every minute. */
export function useAutoUpdateStatus(settings: UpdateSettingsResponse | undefined): string | null {
  return autoUpdateStatus(settings, useNow())
}
