import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { KeyboardEvent as ReactKeyboardEvent, PointerEvent as ReactPointerEvent } from 'react';
import {
  DAYS,
  PRESETS,
  countHours,
  dayLabel,
  fillRect,
  formatHour,
  groupWindows,
  hoursText,
  nowInZone,
  presetSchedule,
  rectLabel,
  removeGroup,
  runs,
  windowStatus,
} from './weekSchedule';
import type { CellRect, LocalNow, WeekSchedule } from './weekSchedule';

type DragKind = 'cell' | 'col' | 'row';

interface Drag {
  kind: DragKind;
  d0: number;
  h0: number;
  d1: number;
  h1: number;
  add: boolean;
}

interface Target {
  kind: DragKind;
  d: number;
  h: number;
}

export interface WeekScheduleGridProps {
  schedule: WeekSchedule;
  onChange: (schedule: WeekSchedule) => void;
  /** IANA zone the schedule is evaluated in; drives the "now" outline. */
  timezone: string;
  /** Automatic updates switched on; the grid is greyed when false. */
  active: boolean;
  readOnly?: boolean;
  /** Fixed clock for tests. */
  now?: Date;
}

function targetOf(el: Element | null): Target | null {
  if (!el) return null;
  const cell = el.closest<HTMLElement>('[data-cell]');
  if (cell) return { kind: 'cell', d: Number(cell.dataset.d), h: Number(cell.dataset.h) };
  const hour = el.closest<HTMLElement>('[data-hour-header]');
  if (hour) return { kind: 'col', d: 0, h: Number(hour.dataset.h) };
  const day = el.closest<HTMLElement>('[data-day-header]');
  if (day) return { kind: 'row', d: Number(day.dataset.d), h: 0 };
  return null;
}

function dragRect(drag: Drag | null): CellRect | null {
  if (!drag) return null;
  const dA = Math.min(drag.d0, drag.d1);
  const dB = Math.max(drag.d0, drag.d1);
  const hA = Math.min(drag.h0, drag.h1);
  const hB = Math.max(drag.h0, drag.h1);
  if (drag.kind === 'col') return { dA: 0, dB: 6, hA, hB };
  if (drag.kind === 'row') return { dA, dB, hA: 0, hB: 23 };
  return { dA, dB, hA, hB };
}

function inRect(r: CellRect | null, d: number, h: number): boolean {
  return r !== null && d >= r.dA && d <= r.dB && h >= r.hA && h <= r.hB;
}

function useLocalNow(timezone: string, fixed?: Date): LocalNow {
  const [tick, setTick] = useState(0);
  useEffect(() => {
    if (fixed) return undefined;
    const id = window.setInterval(() => setTick((t) => t + 1), 60_000);
    return () => window.clearInterval(id);
  }, [fixed]);
  // eslint-disable-next-line react-hooks/exhaustive-deps
  return useMemo(() => nowInZone(timezone, fixed ?? new Date()), [timezone, fixed, tick]);
}

const headerClass = 'rounded text-center select-none';

export function WeekScheduleGrid({ schedule, onChange, timezone, active, readOnly = false, now: fixedNow }: WeekScheduleGridProps) {
  const now = useLocalNow(timezone, fixedNow);
  const [drag, setDrag] = useState<Drag | null>(null);
  const dragRef = useRef<Drag | null>(null);
  const scheduleRef = useRef(schedule);
  scheduleRef.current = schedule;
  const [focus, setFocus] = useState<{ d: number; h: number }>({ d: 0, h: 0 });
  const cellRefs = useRef<(HTMLButtonElement | null)[]>([]);

  const updateDrag = (next: Drag | null) => {
    dragRef.current = next;
    setDrag(next);
  };

  const finish = useCallback((commit: boolean) => {
    const current = dragRef.current;
    if (!current) return;
    dragRef.current = null;
    setDrag(null);
    const rect = dragRect(current);
    if (commit && rect) onChange(fillRect(scheduleRef.current, rect, current.add));
  }, [onChange]);

  // Release anywhere applies the drag; Esc cancels it.
  useEffect(() => {
    if (!drag) return undefined;
    const up = () => finish(true);
    const cancel = () => finish(false);
    const key = (e: KeyboardEvent) => { if (e.key === 'Escape') finish(false); };
    window.addEventListener('pointerup', up);
    window.addEventListener('pointercancel', cancel);
    window.addEventListener('keydown', key);
    return () => {
      window.removeEventListener('pointerup', up);
      window.removeEventListener('pointercancel', cancel);
      window.removeEventListener('keydown', key);
    };
  }, [drag !== null, finish]); // eslint-disable-line react-hooks/exhaustive-deps

  const onPointerDown = (e: ReactPointerEvent<HTMLDivElement>) => {
    if (readOnly || (e.button !== undefined && e.button > 0)) return;
    const t = targetOf(e.target as Element);
    if (!t) return;
    e.preventDefault();
    const s = scheduleRef.current;
    let add: boolean;
    if (t.kind === 'cell') add = !s[t.d][t.h];
    else if (t.kind === 'col') add = !s.every((row) => row[t.h]);
    else add = !s[t.d].every(Boolean);
    if (t.kind === 'cell') setFocus({ d: t.d, h: t.h });
    try {
      e.currentTarget.setPointerCapture?.(e.pointerId);
    } catch {
      // Synthetic events have no active pointer to capture.
    }
    updateDrag({ kind: t.kind, d0: t.d, h0: t.h, d1: t.d, h1: t.h, add });
  };

  const onPointerMove = (e: ReactPointerEvent<HTMLDivElement>) => {
    const current = dragRef.current;
    if (!current) return;
    const under = typeof document.elementFromPoint === 'function' && (e.clientX || e.clientY)
      ? document.elementFromPoint(e.clientX, e.clientY)
      : null;
    const t = targetOf(under) ?? targetOf(e.target as Element);
    if (!t) return;
    // Stay in the mode the drag started in; a cell drag that wanders onto a
    // header keeps tracking that header's hour or day.
    const d1 = current.kind === 'col' || t.kind === 'col' ? current.d1 : t.d;
    const h1 = current.kind === 'row' || t.kind === 'row' ? current.h1 : t.h;
    if (d1 !== current.d1 || h1 !== current.h1) updateDrag({ ...current, d1, h1 });
  };

  const toggle = (d: number, h: number) => {
    onChange(schedule.map((row, di) => row.map((on, hi) => (di === d && hi === h ? !on : on))));
  };

  const onCellKeyDown = (e: ReactKeyboardEvent<HTMLButtonElement>, d: number, h: number) => {
    if (e.key === ' ' || e.key === 'Enter') {
      e.preventDefault();
      if (!readOnly) toggle(d, h);
      return;
    }
    // Navigation works read-only too (roving tabindex); only toggling is blocked.
    const ctrl = e.ctrlKey || e.metaKey;
    let target: [number, number] | null = null;
    if (e.key === 'ArrowLeft') target = [d, h - 1];
    else if (e.key === 'ArrowRight') target = [d, h + 1];
    else if (e.key === 'ArrowUp') target = [d - 1, h];
    else if (e.key === 'ArrowDown') target = [d + 1, h];
    else if (e.key === 'Home') target = ctrl ? [0, 0] : [d, 0];
    else if (e.key === 'End') target = ctrl ? [6, 23] : [d, 23];
    if (!target) return;
    e.preventDefault();
    const nd = Math.min(6, Math.max(0, target[0]));
    const nh = Math.min(23, Math.max(0, target[1]));
    setFocus({ d: nd, h: nh });
    cellRefs.current[nd * 24 + nh]?.focus();
  };

  const rect = dragRect(drag);
  const list = useMemo(() => runs(schedule), [schedule]);
  const groups = useMemo(() => groupWindows(list), [list]);
  const status = useMemo(() => windowStatus(schedule, now), [schedule, now]);
  const total = countHours(schedule);
  const interactive = !readOnly;

  return (
    <div className="space-y-4">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <span className="text-sm font-medium text-gray-700 dark:text-gray-300" id="update-hours-label">Update hours</span>
        <div className="flex flex-wrap gap-2" role="group" aria-label="Presets">
          {PRESETS.map((p) => (
            <button
              key={p.name}
              type="button"
              disabled={readOnly}
              onClick={() => onChange(presetSchedule(p.name))}
              className="px-3 py-1.5 text-sm rounded-full border border-gray-200 dark:border-gray-600 bg-gray-50 dark:bg-gray-700 text-gray-800 dark:text-gray-100 hover:bg-gray-100 dark:hover:bg-gray-600 disabled:opacity-50 disabled:cursor-not-allowed"
            >
              {p.label}
            </button>
          ))}
        </div>
      </div>

      <div className="overflow-x-auto pb-1">
        <div
          role="grid"
          aria-labelledby="update-hours-label"
          aria-readonly={readOnly || undefined}
          data-testid="week-schedule-grid"
          onPointerDown={onPointerDown}
          onPointerMove={onPointerMove}
          className={`grid gap-[3px] select-none ${interactive ? 'touch-none' : ''} ${active ? '' : 'opacity-70'}`}
          style={{ gridTemplateColumns: '3.2rem repeat(24, minmax(26px, 1fr))', minWidth: 760 }}
        >
          <div role="row" className="contents">
            <div role="columnheader" aria-hidden="true" />
            {Array.from({ length: 24 }, (_, h) => (
              <div
                key={h}
                role="columnheader"
                data-hour-header=""
                data-h={h}
                aria-label={`${formatHour(h)}, all days`}
                className={`${headerClass} pb-1 font-mono text-[0.66rem] tabular-nums ${interactive ? 'cursor-pointer hover:bg-gray-100 dark:hover:bg-gray-700 hover:text-gray-900 dark:hover:text-white' : ''} ${h === now.hour ? 'text-amber-600 dark:text-amber-400 font-semibold' : 'text-gray-500 dark:text-gray-400'}`}
              >
                {String(h).padStart(2, '0')}
              </div>
            ))}
          </div>
          {DAYS.map((day, d) => (
            <div role="row" className="contents" key={day}>
              <div
                role="rowheader"
                data-day-header=""
                data-d={d}
                className={`${headerClass} text-left text-xs font-semibold leading-7 ${interactive ? 'cursor-pointer hover:bg-gray-100 dark:hover:bg-gray-700' : ''} ${d === now.day ? 'text-gray-900 dark:text-white' : 'text-gray-500 dark:text-gray-400'}`}
              >
                {day}
              </div>
              {Array.from({ length: 24 }, (_, h) => {
                const hit = inRect(rect, d, h);
                const on = hit && drag ? drag.add : schedule[d][h];
                const isNow = d === now.day && h === now.hour;
                const onClass = active ? 'bg-blue-600 dark:bg-blue-500' : 'bg-blue-300 dark:bg-blue-900';
                return (
                  <button
                    key={h}
                    ref={(el) => { cellRefs.current[d * 24 + h] = el; }}
                    type="button"
                    role="gridcell"
                    data-cell=""
                    data-d={d}
                    data-h={h}
                    data-preview={hit || undefined}
                    data-now={isNow || undefined}
                    aria-label={`${day} ${formatHour(h)}`}
                    aria-selected={schedule[d][h]}
                    aria-disabled={readOnly || undefined}
                    tabIndex={focus.d === d && focus.h === h ? 0 : -1}
                    onFocus={() => setFocus({ d, h })}
                    onKeyDown={(e) => onCellKeyDown(e, d, h)}
                    className={`h-7 rounded-[5px] p-0 border-0 ${on ? onClass : 'bg-gray-100 dark:bg-gray-700'} ${interactive ? 'cursor-pointer' : 'cursor-default'} ${interactive && !on ? 'hover:bg-gray-200 dark:hover:bg-gray-600' : ''} ${hit ? 'opacity-85 shadow-[inset_0_0_0_2px_rgb(17_24_39)] dark:shadow-[inset_0_0_0_2px_white]' : ''} ${isNow ? 'outline-2 outline-offset-1 outline-amber-500 dark:outline-amber-400' : ''} focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-blue-500`}
                  />
                );
              })}
            </div>
          ))}
        </div>
      </div>

      {drag && rect && (
        <span data-testid="drag-label" className="inline-block rounded-full border border-gray-200 dark:border-gray-600 bg-gray-50 dark:bg-gray-700 px-3 py-1 font-mono text-xs font-semibold text-gray-900 dark:text-white">
          {rectLabel(rect, drag.add)}
        </span>
      )}

      <div className="flex flex-wrap gap-4 text-xs text-gray-500 dark:text-gray-400">
        <span className="inline-flex items-center gap-1.5"><i className="inline-block h-3.5 w-3.5 rounded bg-blue-600 dark:bg-blue-500" />Updates may start</span>
        <span className="inline-flex items-center gap-1.5"><i className="inline-block h-3.5 w-3.5 rounded bg-gray-100 dark:bg-gray-700" />No updates</span>
        <span className="inline-flex items-center gap-1.5"><i className="inline-block h-3.5 w-3.5 rounded shadow-[inset_0_0_0_2px_var(--color-amber-500)]" />Now ({timezone})</span>
      </div>

      <div>
        <div className="flex flex-wrap items-baseline justify-between gap-2 mb-2">
          <span className="text-sm font-medium text-gray-700 dark:text-gray-300">Your update windows</span>
          <span className="font-mono text-xs text-gray-500 dark:text-gray-400">
            {total ? `${total} h a week in ${list.length} window${list.length === 1 ? '' : 's'}` : 'No hours selected'}
          </span>
        </div>
        {groups.length === 0 ? (
          <p className="text-sm text-gray-500 dark:text-gray-400">No windows yet. Paint hours in the grid or pick a preset.</p>
        ) : (
          <ul className="grid gap-2" aria-label="Update windows">
            {groups.map((g) => (
              <WindowRow
                key={`${g.startHour}:${g.len}`}
                group={g}
                isOpen={status.openStart !== null && g.starts.includes(status.openStart)}
                isNext={status.nextStart !== null && g.starts.includes(status.nextStart)}
                onRemove={readOnly ? undefined : () => onChange(removeGroup(schedule, g))}
              />
            ))}
          </ul>
        )}
      </div>
    </div>
  );
}

function WindowRow({ group: g, isOpen, isNext, onRemove }: {
  group: ReturnType<typeof groupWindows>[number];
  isOpen: boolean;
  isNext: boolean;
  onRemove?: () => void;
}) {
  const label = dayLabel(g.days);
  const endHour = (g.startHour + g.len) % 24;
  const overnight = g.startHour + g.len > 24;
  // Up to 48h so overnight windows stay one block.
  const span = overnight ? 48 : 24;
  const left = (g.startHour / span) * 100;
  const width = (Math.min(g.len, span - g.startHour) / span) * 100;
  const ticks = overnight ? [0, 12, 24, 36, 48] : [0, 6, 12, 18, 24];
  const range = `${formatHour(g.startHour)} – ${formatHour(endHour)}`;
  return (
    <li
      className={`grid grid-cols-[1fr_auto] sm:grid-cols-[minmax(7.5rem,10rem)_minmax(8.5rem,11rem)_1fr_auto] items-center gap-x-4 gap-y-1.5 rounded-lg border px-3 py-2.5 bg-gray-50 dark:bg-gray-900/40 ${isOpen || isNext ? 'border-blue-500 shadow-[inset_3px_0_0_var(--color-blue-500)]' : 'border-gray-200 dark:border-gray-700'}`}
    >
      <div className="font-semibold text-gray-900 dark:text-white">
        {label}
        <small className="block text-xs font-medium text-gray-500 dark:text-gray-400">
          {g.days.length === 1 ? 'once a week' : `${g.days.length} days a week`}
        </small>
      </div>
      <div className="row-start-2 sm:row-start-auto font-mono text-sm font-semibold tabular-nums text-gray-900 dark:text-white">
        {range}
        <small className="block font-sans text-xs font-medium text-gray-500 dark:text-gray-400">
          {hoursText(g.len)}{overnight ? ', ends the next day' : ''}
        </small>
      </div>
      <div className="col-span-2 row-start-3 sm:col-span-1 sm:row-start-auto min-w-0 pb-4">
        <div className="relative h-[18px] rounded-[5px] bg-gray-200 dark:bg-gray-700" role="img" aria-label={`${formatHour(g.startHour)} to ${formatHour(endHour)}`}>
          <i className="absolute inset-y-0 rounded-[5px] bg-blue-600 dark:bg-blue-500" style={{ left: `${left}%`, width: `${width}%` }} />
          {ticks.map((t) => (
            <span key={t} className="absolute top-full -translate-x-1/2 font-mono text-[0.6rem] leading-relaxed text-gray-500 dark:text-gray-400" style={{ left: `${(t / span) * 100}%` }}>
              {String(t % 24).padStart(2, '0')}
            </span>
          ))}
        </div>
      </div>
      <div className="col-start-2 row-span-2 row-start-1 sm:col-start-auto sm:row-span-1 sm:row-start-auto flex items-center justify-end gap-2">
        {isOpen && <span className="whitespace-nowrap rounded-full border border-blue-500 px-2 py-0.5 font-mono text-[0.66rem] font-semibold uppercase tracking-wide text-blue-600 dark:text-blue-400">Open now</span>}
        {!isOpen && isNext && <span className="whitespace-nowrap rounded-full border border-blue-500 px-2 py-0.5 font-mono text-[0.66rem] font-semibold uppercase tracking-wide text-blue-600 dark:text-blue-400">Next</span>}
        {onRemove && (
          <button
            type="button"
            onClick={onRemove}
            aria-label={`Remove ${label} ${formatHour(g.startHour)} to ${formatHour(endHour)}`}
            className="h-7 w-7 rounded-lg border border-gray-200 dark:border-gray-600 text-gray-500 hover:bg-gray-100 dark:hover:bg-gray-700 hover:text-gray-900 dark:hover:text-white"
          >
            ×
          </button>
        )}
      </div>
    </li>
  );
}
