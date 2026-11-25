import type { UpdateSettingsResponse } from '../api/updates'

interface ZonedParts {
  year: number
  month: number
  day: number
  hour: number
  minute: number
  weekday: string
}

function zoned(date: Date, timeZone: string): ZonedParts {
  const parts = new Intl.DateTimeFormat('en-US', {
    timeZone, year: 'numeric', month: '2-digit', day: '2-digit',
    hour: '2-digit', minute: '2-digit', hourCycle: 'h23', weekday: 'long',
  }).formatToParts(date)
  const get = (type: string) => parts.find((p) => p.type === type)?.value ?? ''
  return {
    year: Number(get('year')),
    month: Number(get('month')),
    day: Number(get('day')),
    // Some engines render midnight as "24" even with h23.
    hour: Number(get('hour')) % 24,
    minute: Number(get('minute')),
    weekday: get('weekday'),
  }
}

/** The card's auto-update status line; null when the viewer cannot see the settings. */
export function autoUpdateStatus(settings: UpdateSettingsResponse | undefined, now: Date = new Date()): string | null {
  if (!settings) return null
  if (!settings.enabled) return 'Automatic updates are off'
  const hours = settings.schedule?.flat() ?? []
  if (!hours.some(Boolean) || !settings.next_window_start) return 'No update hours selected'
  if (hours.length === 168 && hours.every(Boolean)) return 'Updates can start any time'

  const start = new Date(settings.next_window_start)
  if (Number.isNaN(start.getTime())) return null
  let zone = settings.timezone
  let at: ZonedParts
  let today: ZonedParts
  try {
    at = zoned(start, zone)
    today = zoned(now, zone)
  } catch {
    zone = 'UTC'
    at = zoned(start, zone)
    today = zoned(now, zone)
  }
  const time = `${String(at.hour).padStart(2, '0')}:${String(at.minute).padStart(2, '0')}`
  if (start.getTime() <= now.getTime()) return `Auto-update window open now (${zone})`

  const days = Math.round(
    (Date.UTC(at.year, at.month - 1, at.day) - Date.UTC(today.year, today.month - 1, today.day)) / 86_400_000,
  )
  const night = at.hour < 6 || at.hour >= 18
  let when: string
  if (days === 0) when = night ? 'tonight' : 'today'
  else if (days === 1 && at.hour < 6) when = 'tonight'
  else if (days === 1) when = 'tomorrow'
  else when = `on ${at.weekday}`
  return `Auto-update ${when} at ${time} (${zone})`
}
