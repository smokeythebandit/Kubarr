import { describe, expect, it } from 'vitest';
import {
  countHours,
  dayLabel,
  emptySchedule,
  fillRect,
  groupWindows,
  isFull,
  normalizeSchedule,
  nowInZone,
  presetSchedule,
  rectLabel,
  removeGroup,
  runs,
  windowStatus,
} from './weekSchedule';

const at = (list: [number, number][]) => {
  const s = emptySchedule();
  for (const [d, h] of list) s[d][h] = true;
  return s;
};

describe('weekSchedule helpers', () => {
  it('builds the presets', () => {
    expect(countHours(presetSchedule('nightly'))).toBe(14);
    expect(countHours(presetSchedule('late'))).toBe(35);
    const weekend = presetSchedule('weekend');
    expect(countHours(weekend)).toBe(12);
    expect(weekend[5][1] && weekend[6][6] && !weekend[6][7] && !weekend[4][1]).toBe(true);
    expect(countHours(presetSchedule('workday'))).toBe(30);
    expect(countHours(presetSchedule('clear'))).toBe(0);
  });

  it('normalizes malformed input to 7x24', () => {
    const s = normalizeSchedule([[true]]);
    expect(s).toHaveLength(7);
    expect(s.every((r) => r.length === 24)).toBe(true);
    expect(countHours(s)).toBe(1);
  });

  it('finds runs and wraps Sunday night into Monday', () => {
    expect(runs(emptySchedule())).toEqual([]);
    expect(runs(at([[2, 3], [2, 4]]))).toEqual([{ start: 51, len: 2 }]);
    expect(runs(at([[6, 23], [0, 0], [0, 1]]))).toEqual([{ start: 167, len: 3 }]);
    expect(runs(at([[1, 23], [2, 0]]))).toEqual([{ start: 47, len: 2 }]);
    const full = emptySchedule().map((r) => r.map(() => true));
    expect(isFull(full)).toBe(true);
    expect(runs(full)).toEqual([{ start: 0, len: 168 }]);
  });

  it('groups identical windows across days', () => {
    const groups = groupWindows(runs(presetSchedule('late')));
    expect(groups).toEqual([{ startHour: 1, len: 5, days: [0, 1, 2, 3, 4, 5, 6], starts: [1, 25, 49, 73, 97, 121, 145] }]);
    const mixed = groupWindows(runs(fillRect(presetSchedule('workday'), { dA: 5, dB: 6, hA: 1, hB: 6 }, true)));
    expect(mixed.map((g) => [g.startHour, g.len, dayLabel(g.days)])).toEqual([[10, 6, 'Weekdays'], [1, 6, 'Weekends']]);
  });

  it('labels days', () => {
    expect(dayLabel([0, 1, 2, 3, 4, 5, 6])).toBe('Every day');
    expect(dayLabel([4, 3, 2, 1, 0])).toBe('Weekdays');
    expect(dayLabel([5, 6])).toBe('Weekends');
    expect(dayLabel([0, 1, 2, 3])).toBe('Mon–Thu');
    expect(dayLabel([0, 2])).toBe('Mon, Wed');
    expect(dayLabel([0, 1])).toBe('Mon, Tue');
  });

  it('reports the open or next window', () => {
    const s = presetSchedule('nightly');
    expect(windowStatus(s, { day: 2, hour: 4, minute: 30 })).toEqual({ openStart: 52, nextStart: null });
    expect(windowStatus(s, { day: 2, hour: 12, minute: 0 })).toEqual({ openStart: null, nextStart: 76 });
    // From Sunday afternoon the next start wraps to Monday.
    expect(windowStatus(at([[0, 4]]), { day: 6, hour: 12, minute: 0 })).toEqual({ openStart: null, nextStart: 4 });
    expect(windowStatus(emptySchedule(), { day: 0, hour: 0, minute: 0 })).toEqual({ openStart: null, nextStart: null });
  });

  it('removes every run of a group, across the week wrap', () => {
    const s = at([[6, 23], [0, 0], [2, 4]]);
    const [wrap] = groupWindows(runs(s)).filter((g) => g.len === 2);
    const left = removeGroup(s, wrap);
    expect(countHours(left)).toBe(1);
    expect(left[2][4]).toBe(true);
  });

  it('describes a drag rectangle', () => {
    expect(rectLabel({ dA: 0, dB: 6, hA: 1, hB: 5 }, true)).toBe('Add 01:00–06:00 · Mon–Sun · 35 h');
    expect(rectLabel({ dA: 2, dB: 2, hA: 23, hB: 23 }, false)).toBe('Remove 23:00–00:00 · Wed · 1 h');
  });

  it('reads the local weekday and hour in a zone', () => {
    // Wednesday 23:30 UTC is Thursday 00:30 in Amsterdam.
    expect(nowInZone('Europe/Amsterdam', new Date('2026-11-25T23:30:00Z'))).toEqual({ day: 3, hour: 0, minute: 30 });
    expect(nowInZone('UTC', new Date('2026-11-30T00:05:00Z'))).toEqual({ day: 0, hour: 0, minute: 5 });
  });
});
