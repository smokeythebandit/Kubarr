import type { UpdateSettingsResponse } from '../api/updates'

/** The card's auto-update status line; null when the viewer cannot see the settings. */
export function autoUpdateStatus(settings: UpdateSettingsResponse | undefined): string | null {
  if (!settings) return null
  if (!settings.enabled) return 'Automatic updates are off'
  if (!settings.next_window_start) return null
  let time: string
  try {
    time = new Date(settings.next_window_start).toLocaleTimeString([], {
      hour: '2-digit', minute: '2-digit', hour12: false, timeZone: settings.timezone,
    })
  } catch {
    time = settings.window_start
  }
  return `Auto-update tonight at ${time}`
}
