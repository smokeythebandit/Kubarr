import type { QuietHours } from '../../api/notifications';

export const QUIET_HOURS_DESCRIPTION =
  'Hold back email, Telegram and SMS at night. They still land in the bell, and are sent together when quiet hours end.';

export function browserTimezone(): string {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone || 'UTC';
  } catch {
    return 'UTC';
  }
}

export function timezoneNames(extra: string[]): string[] {
  let names: string[] = [];
  try {
    const supported = (Intl as unknown as { supportedValuesOf?: (key: string) => string[] }).supportedValuesOf;
    names = supported ? supported('timeZone') : [];
  } catch {
    names = [];
  }
  const all = new Set(['UTC', ...names, ...extra]);
  return [...all].sort((a, b) => (a === 'UTC' ? -1 : b === 'UTC' ? 1 : a.localeCompare(b)));
}

/** 420 → "07:00". */
export function minutesToTime(minutes: number): string {
  const h = Math.floor(minutes / 60);
  const m = minutes % 60;
  return `${String(h).padStart(2, '0')}:${String(m).padStart(2, '0')}`;
}

/** "07:00" → 420; null for anything else. */
export function timeToMinutes(value: string): number | null {
  const match = /^(\d{2}):(\d{2})$/.exec(value);
  if (!match) return null;
  const h = Number(match[1]);
  const m = Number(match[2]);
  if (h > 23 || m > 59) return null;
  return h * 60 + m;
}

/** The one-line explanation under the controls. */
export function quietHoursPreview(q: Pick<QuietHours, 'enabled' | 'from_minute' | 'to_minute' | 'timezone' | 'critical_bypass'>): string {
  if (!q.enabled) return 'Quiet hours are off: email, Telegram and SMS go out straight away.';
  const from = minutesToTime(q.from_minute);
  const to = minutesToTime(q.to_minute);
  const critical = q.critical_bypass ? 'critical notifications still come through straight away' : 'critical notifications wait too';
  return `Quiet from ${from} to ${to} ${q.timezone}; held messages arrive as one summary at ${to}, ${critical}.`;
}

