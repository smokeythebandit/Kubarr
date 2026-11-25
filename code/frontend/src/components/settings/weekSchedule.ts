/** A week of update hours: 7 rows (Monday first) of 24 booleans. */
export type WeekSchedule = boolean[][];

export const DAYS = ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun'] as const;
export const HOURS_PER_WEEK = 168;

/** A contiguous run of enabled hours; `start` is the hour of the week (0 = Monday 00:00). */
export interface Run {
  start: number;
  len: number;
}

/** Runs with the same start hour and length on several days. */
export interface WindowGroup {
  startHour: number;
  len: number;
  days: number[];
  starts: number[];
}

/** An inclusive block of days and hours. */
export interface CellRect {
  dA: number;
  dB: number;
  hA: number;
  hB: number;
}

export interface LocalNow {
  day: number;
  hour: number;
  minute: number;
}

export type PresetName = 'nightly' | 'late' | 'weekend' | 'workday' | 'clear';

export const PRESETS: { name: PresetName; label: string }[] = [
  { name: 'nightly', label: 'Every night 04–06' },
  { name: 'late', label: 'Every night 01–06' },
  { name: 'weekend', label: 'Weekend nights only' },
  { name: 'workday', label: 'Weekdays 10–16' },
  { name: 'clear', label: 'Clear' },
];

export function emptySchedule(): WeekSchedule {
  return Array.from({ length: 7 }, () => Array<boolean>(24).fill(false));
}

/** A copy padded/trimmed to 7×24 so malformed input never breaks the grid. */
export function normalizeSchedule(input: boolean[][] | null | undefined): WeekSchedule {
  return Array.from({ length: 7 }, (_, d) =>
    Array.from({ length: 24 }, (_, h) => Boolean(input?.[d]?.[h])));
}

export function fillRect(schedule: WeekSchedule, rect: CellRect, value: boolean): WeekSchedule {
  return schedule.map((row, d) => row.map((on, h) =>
    d >= rect.dA && d <= rect.dB && h >= rect.hA && h <= rect.hB ? value : on));
}

export function presetSchedule(name: PresetName): WeekSchedule {
  const s = emptySchedule();
  const set = (days: number[], from: number, to: number) => {
    for (const d of days) for (let h = from; h < to; h++) s[d][h] = true;
  };
  const all = [0, 1, 2, 3, 4, 5, 6];
  if (name === 'nightly') set(all, 4, 6);
  if (name === 'late') set(all, 1, 6);
  if (name === 'weekend') set([5, 6], 1, 7);
  if (name === 'workday') set([0, 1, 2, 3, 4], 10, 16);
  return s;
}

export function countHours(schedule: WeekSchedule): number {
  return schedule.flat().filter(Boolean).length;
}

export function isFull(schedule: WeekSchedule): boolean {
  return countHours(schedule) === HOURS_PER_WEEK;
}

/** Contiguous enabled hours across the week; Sunday night wraps into Monday. */
export function runs(schedule: WeekSchedule): Run[] {
  const flat = schedule.flat();
  if (flat.length !== HOURS_PER_WEEK) return [];
  if (flat.every(Boolean)) return [{ start: 0, len: HOURS_PER_WEEK }];
  // Begin scanning just after an off hour so no run is split by the wrap.
  const off = flat.findIndex((v) => !v);
  if (off < 0 || !flat.some(Boolean)) return [];
  const out: Run[] = [];
  for (let k = 1; k <= HOURS_PER_WEEK; k++) {
    const i = (off + k) % HOURS_PER_WEEK;
    if (!flat[i]) continue;
    if (!flat[(i + HOURS_PER_WEEK - 1) % HOURS_PER_WEEK]) out.push({ start: i, len: 0 });
    out[out.length - 1].len++;
  }
  return out.sort((a, b) => a.start - b.start);
}

/** Group runs that start at the same hour of day and last as long. */
export function groupWindows(list: Run[]): WindowGroup[] {
  const groups = new Map<string, WindowGroup>();
  for (const w of list) {
    const key = `${w.start % 24}:${w.len}`;
    let g = groups.get(key);
    if (!g) {
      g = { startHour: w.start % 24, len: w.len, days: [], starts: [] };
      groups.set(key, g);
    }
    g.days.push(Math.floor(w.start / 24));
    g.starts.push(w.start);
  }
  const out = [...groups.values()];
  for (const g of out) g.days.sort((a, b) => a - b);
  return out.sort((a, b) => a.days[0] - b.days[0] || a.startHour - b.startHour);
}

/** "Every day", "Weekdays", "Weekends", "Mon–Thu" or "Mon, Wed". */
export function dayLabel(days: number[]): string {
  const sorted = [...days].sort((a, b) => a - b);
  const key = sorted.join(',');
  if (key === '0,1,2,3,4,5,6') return 'Every day';
  if (key === '0,1,2,3,4') return 'Weekdays';
  if (key === '5,6') return 'Weekends';
  const contiguous = sorted.every((d, i) => i === 0 || d === sorted[i - 1] + 1);
  if (contiguous && sorted.length > 2) return `${DAYS[sorted[0]]}–${DAYS[sorted[sorted.length - 1]]}`;
  return sorted.map((d) => DAYS[d]).join(', ');
}

export function formatHour(hourOfWeek: number): string {
  return `${String(((hourOfWeek % 24) + 24) % 24).padStart(2, '0')}:00`;
}

export function hoursText(n: number): string {
  return n === 1 ? '1 hour' : `${n} hours`;
}

/** Weekday (Monday = 0), hour and minute of `date` in `timezone`. */
export function nowInZone(timezone: string, date: Date = new Date()): LocalNow {
  const read = (tz: string | undefined) => {
    const parts = new Intl.DateTimeFormat('en-GB', {
      timeZone: tz, weekday: 'short', hour: '2-digit', minute: '2-digit', hourCycle: 'h23',
    }).formatToParts(date);
    const get = (type: string) => parts.find((p) => p.type === type)?.value ?? '';
    return {
      day: DAYS.indexOf(get('weekday') as (typeof DAYS)[number]),
      hour: Number(get('hour')) % 24,
      minute: Number(get('minute')),
    };
  };
  try {
    return read(timezone);
  } catch {
    return read(undefined);
  }
}

/**
 * The run containing `now` (`openStart`) or, when closed, the hour of the
 * week the next one starts (`nextStart`).
 */
export function windowStatus(schedule: WeekSchedule, now: LocalNow): { openStart: number | null; nextStart: number | null } {
  const flat = schedule.flat();
  const nowH = now.day * 24 + now.hour;
  if (flat[nowH]) {
    const open = runs(schedule).find((w) => (nowH - w.start + HOURS_PER_WEEK) % HOURS_PER_WEEK < w.len);
    return { openStart: open?.start ?? null, nextStart: null };
  }
  for (let k = 1; k < HOURS_PER_WEEK; k++) {
    const i = (nowH + k) % HOURS_PER_WEEK;
    if (flat[i]) return { openStart: null, nextStart: i };
  }
  return { openStart: null, nextStart: null };
}

/** "Add 01:00–06:00 · Mon–Sun · 35 h" */
export function rectLabel(rect: CellRect, add: boolean): string {
  const n = (rect.dB - rect.dA + 1) * (rect.hB - rect.hA + 1);
  const days = rect.dA === rect.dB ? DAYS[rect.dA] : `${DAYS[rect.dA]}–${DAYS[rect.dB]}`;
  return `${add ? 'Add' : 'Remove'} ${formatHour(rect.hA)}–${formatHour(rect.hB + 1)} · ${days} · ${n} h`;
}

/** Turn off every hour of a group's runs. */
export function removeGroup(schedule: WeekSchedule, group: WindowGroup): WeekSchedule {
  const next = schedule.map((row) => [...row]);
  for (const s of group.starts) {
    for (let k = 0; k < group.len; k++) {
      const i = (s + k) % HOURS_PER_WEEK;
      next[Math.floor(i / 24)][i % 24] = false;
    }
  }
  return next;
}
