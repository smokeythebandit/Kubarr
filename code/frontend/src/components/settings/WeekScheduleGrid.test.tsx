import { useState } from 'react';
import { describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, within } from '@testing-library/react';
import { WeekScheduleGrid } from './WeekScheduleGrid';
import { countHours, emptySchedule, presetSchedule } from './weekSchedule';
import type { WeekSchedule } from './weekSchedule';

// Wednesday 2026-11-25 12:30 UTC.
const NOW = new Date('2026-11-25T12:30:00Z');

function Harness({ initial, onChange, readOnly = false, active = true }: {
  initial: WeekSchedule;
  onChange?: (s: WeekSchedule) => void;
  readOnly?: boolean;
  active?: boolean;
}) {
  const [schedule, setSchedule] = useState(initial);
  return (
    <WeekScheduleGrid
      schedule={schedule}
      onChange={(s) => { setSchedule(s); onChange?.(s); }}
      timezone="UTC"
      active={active}
      readOnly={readOnly}
      now={NOW}
    />
  );
}

const cell = (day: string, hour: number) => screen.getByRole('gridcell', { name: `${day} ${String(hour).padStart(2, '0')}:00` });
const selected = (day: string, hour: number) => cell(day, hour).getAttribute('aria-selected') === 'true';

describe('WeekScheduleGrid', () => {
  it('applies a preset', () => {
    const onChange = vi.fn();
    render(<Harness initial={emptySchedule()} onChange={onChange} />);
    fireEvent.click(screen.getByRole('button', { name: 'Weekend nights only' }));
    expect(countHours(onChange.mock.calls[0][0])).toBe(12);
    expect(selected('Sat', 1) && selected('Sun', 6)).toBe(true);
    expect(selected('Fri', 1)).toBe(false);
    expect(screen.getByText('Weekends')).toBeVisible();
    expect(screen.getByText('01:00 – 07:00')).toBeVisible();
  });

  it('applies a rectangle drag only on pointer release', () => {
    const onChange = vi.fn();
    render(<Harness initial={emptySchedule()} onChange={onChange} />);
    fireEvent.pointerDown(cell('Mon', 1));
    fireEvent.pointerMove(cell('Wed', 3));
    expect(onChange).not.toHaveBeenCalled();
    expect(screen.getByTestId('drag-label')).toHaveTextContent('Add 01:00–04:00 · Mon–Wed · 9 h');
    expect(cell('Tue', 2)).toHaveAttribute('data-preview', 'true');
    expect(selected('Tue', 2)).toBe(false);
    fireEvent.pointerUp(cell('Wed', 3));
    expect(onChange).toHaveBeenCalledTimes(1);
    expect(countHours(onChange.mock.calls[0][0])).toBe(9);
    expect(selected('Tue', 2) && selected('Mon', 1) && selected('Wed', 3)).toBe(true);
    expect(selected('Thu', 2)).toBe(false);
    expect(screen.queryByTestId('drag-label')).not.toBeInTheDocument();
  });

  it('selects whole columns when dragging over hour headers', () => {
    const onChange = vi.fn();
    render(<Harness initial={emptySchedule()} onChange={onChange} />);
    fireEvent.pointerDown(screen.getByRole('columnheader', { name: '02:00, all days' }));
    fireEvent.pointerMove(screen.getByRole('columnheader', { name: '03:00, all days' }));
    fireEvent.pointerUp(document.body);
    expect(countHours(onChange.mock.calls[0][0])).toBe(14);
    expect(selected('Mon', 2) && selected('Sun', 3)).toBe(true);
    expect(screen.getByText('Every day')).toBeVisible();
  });

  it('selects whole rows when dragging over day names', () => {
    const onChange = vi.fn();
    render(<Harness initial={emptySchedule()} onChange={onChange} />);
    fireEvent.pointerDown(screen.getByRole('rowheader', { name: 'Sat' }));
    fireEvent.pointerMove(screen.getByRole('rowheader', { name: 'Sun' }));
    fireEvent.pointerUp(document.body);
    expect(countHours(onChange.mock.calls[0][0])).toBe(48);
  });

  it('removes when the drag starts on an enabled hour', () => {
    const onChange = vi.fn();
    render(<Harness initial={presetSchedule('late')} onChange={onChange} />);
    fireEvent.pointerDown(cell('Mon', 2));
    fireEvent.pointerMove(cell('Sun', 5));
    expect(screen.getByTestId('drag-label')).toHaveTextContent('Remove 02:00–06:00 · Mon–Sun · 28 h');
    fireEvent.pointerUp(cell('Sun', 5));
    const result = onChange.mock.calls[0][0] as WeekSchedule;
    expect(countHours(result)).toBe(7);
    expect(result.every((row) => row[1] && !row[2])).toBe(true);
  });

  it('cancels a drag on Escape', () => {
    const onChange = vi.fn();
    render(<Harness initial={emptySchedule()} onChange={onChange} />);
    fireEvent.pointerDown(cell('Mon', 1));
    fireEvent.pointerMove(cell('Tue', 4));
    fireEvent.keyDown(window, { key: 'Escape' });
    expect(screen.queryByTestId('drag-label')).not.toBeInTheDocument();
    fireEvent.pointerUp(cell('Tue', 4));
    expect(onChange).not.toHaveBeenCalled();
    expect(selected('Mon', 1)).toBe(false);
  });

  it('toggles a focused cell with Space and Enter', () => {
    render(<Harness initial={emptySchedule()} />);
    fireEvent.keyDown(cell('Thu', 5), { key: ' ' });
    expect(selected('Thu', 5)).toBe(true);
    fireEvent.keyDown(cell('Thu', 5), { key: 'Enter' });
    expect(selected('Thu', 5)).toBe(false);
  });

  it('outlines the current hour and marks the open window', () => {
    render(<Harness initial={presetSchedule('workday')} />);
    expect(cell('Wed', 12)).toHaveAttribute('data-now', 'true');
    expect(within(screen.getByRole('list', { name: 'Update windows' })).getByText('Open now')).toBeVisible();
  });

  it('lists overnight windows and removes a group with ×', () => {
    const s = emptySchedule();
    for (const row of s) { row[22] = true; row[23] = true; }
    for (const row of s) { row[0] = true; }
    const onChange = vi.fn();
    render(<Harness initial={s} onChange={onChange} />);
    expect(screen.getByText('22:00 – 01:00')).toBeVisible();
    expect(screen.getByText('3 hours, ends the next day')).toBeVisible();
    fireEvent.click(screen.getByRole('button', { name: 'Remove Every day 22:00 to 01:00' }));
    expect(countHours(onChange.mock.calls[0][0])).toBe(0);
  });

  it('is read-only without manage permission', () => {
    const onChange = vi.fn();
    render(<Harness initial={presetSchedule('nightly')} onChange={onChange} readOnly />);
    expect(screen.getByRole('button', { name: 'Clear' })).toBeDisabled();
    fireEvent.pointerDown(cell('Mon', 1));
    fireEvent.pointerUp(cell('Mon', 1));
    fireEvent.keyDown(cell('Mon', 1), { key: ' ' });
    expect(onChange).not.toHaveBeenCalled();
    expect(screen.queryByRole('button', { name: /^Remove / })).not.toBeInTheDocument();
  });

  it('moves focus with arrows, Home/End and Ctrl+Home/End', () => {
    render(<Harness initial={emptySchedule()} />);
    cell('Wed', 5).focus();
    fireEvent.keyDown(cell('Wed', 5), { key: 'ArrowRight' });
    expect(cell('Wed', 6)).toHaveFocus();
    fireEvent.keyDown(cell('Wed', 6), { key: 'ArrowDown' });
    expect(cell('Thu', 6)).toHaveFocus();
    fireEvent.keyDown(cell('Thu', 6), { key: 'Home' });
    expect(cell('Thu', 0)).toHaveFocus();
    fireEvent.keyDown(cell('Thu', 0), { key: 'End' });
    expect(cell('Thu', 23)).toHaveFocus();
    fireEvent.keyDown(cell('Thu', 23), { key: 'Home', ctrlKey: true });
    expect(cell('Mon', 0)).toHaveFocus();
    expect(cell('Mon', 0)).toHaveAttribute('tabindex', '0');
    fireEvent.keyDown(cell('Mon', 0), { key: 'End', ctrlKey: true });
    expect(cell('Sun', 23)).toHaveFocus();
    expect(cell('Mon', 0)).toHaveAttribute('tabindex', '-1');
  });

  it('keeps keyboard navigation when read-only but blocks toggling', () => {
    const onChange = vi.fn();
    render(<Harness initial={emptySchedule()} onChange={onChange} readOnly />);
    cell('Mon', 0).focus();
    fireEvent.keyDown(cell('Mon', 0), { key: 'ArrowRight' });
    expect(cell('Mon', 1)).toHaveFocus();
    expect(cell('Mon', 1)).toHaveAttribute('tabindex', '0');
    fireEvent.keyDown(cell('Mon', 1), { key: 'End' });
    expect(cell('Mon', 23)).toHaveFocus();
    fireEvent.keyDown(cell('Mon', 23), { key: 'Enter' });
    fireEvent.keyDown(cell('Mon', 23), { key: ' ' });
    expect(onChange).not.toHaveBeenCalled();
    expect(selected('Mon', 23)).toBe(false);
  });
});
