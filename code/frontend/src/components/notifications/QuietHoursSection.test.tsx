import { beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import type { QuietHours } from '../../api/notifications';
import QuietHoursSection from './QuietHoursSection';
import { minutesToTime, quietHoursPreview, timeToMinutes } from './quietHoursModel';

vi.mock('../../api/notifications', () => ({
  notificationsApi: {
    getQuietHours: vi.fn(),
    updateQuietHours: vi.fn(),
  },
}));

import { notificationsApi } from '../../api/notifications';

const api = vi.mocked(notificationsApi);

const quiet = (overrides: Partial<QuietHours> = {}): QuietHours => ({
  enabled: true,
  from_minute: 23 * 60,
  to_minute: 7 * 60,
  timezone: 'Europe/Amsterdam',
  critical_bypass: true,
  configured: true,
  ...overrides,
});

beforeEach(() => {
  vi.clearAllMocks();
});

describe('quiet hours helpers', () => {
  it('converts between minutes and HH:MM', () => {
    expect(minutesToTime(0)).toBe('00:00');
    expect(minutesToTime(7 * 60 + 5)).toBe('07:05');
    expect(timeToMinutes('23:59')).toBe(1439);
    expect(timeToMinutes('24:00')).toBeNull();
    expect(timeToMinutes('')).toBeNull();
  });

  it('describes the window, the summary time and critical handling', () => {
    expect(quietHoursPreview(quiet())).toBe(
      'Quiet from 23:00 to 07:00 Europe/Amsterdam; held messages arrive as one summary at 07:00, critical notifications still come through straight away.',
    );
    expect(quietHoursPreview(quiet({ critical_bypass: false }))).toContain('critical notifications wait too');
    expect(quietHoursPreview(quiet({ enabled: false }))).toContain('Quiet hours are off');
  });
});

describe('QuietHoursSection', () => {
  it('shows saved settings and the preview', async () => {
    api.getQuietHours.mockResolvedValue(quiet());
    render(<QuietHoursSection />);
    expect(await screen.findByTestId('quiet-hours-preview')).toHaveTextContent('Quiet from 23:00 to 07:00 Europe/Amsterdam');
    expect(screen.getByRole('switch', { name: 'Quiet hours' })).toHaveAttribute('aria-checked', 'true');
    expect(screen.getByRole('switch', { name: 'Critical notifications always come through' })).toHaveAttribute('aria-checked', 'true');
    expect(screen.getByLabelText('From')).toHaveValue('23:00');
    expect(screen.getByLabelText('to')).toHaveValue('07:00');
    expect(screen.getByRole('combobox', { name: 'Time zone' })).toHaveValue('Europe/Amsterdam');
    // Nothing changed yet.
    expect(screen.getByRole('button', { name: 'Save quiet hours' })).toBeDisabled();
  });

  it('defaults the time zone to the browser zone before the first save', async () => {
    const zone = Intl.DateTimeFormat().resolvedOptions().timeZone || 'UTC';
    api.getQuietHours.mockResolvedValue(quiet({ configured: false, enabled: false, timezone: 'UTC' }));
    render(<QuietHoursSection />);
    expect(await screen.findByRole('combobox', { name: 'Time zone' })).toHaveValue(zone);
    expect(screen.getByRole('button', { name: 'Save quiet hours' })).toBeEnabled();
  });

  it('saves changes', async () => {
    api.getQuietHours.mockResolvedValue(quiet({ enabled: false }));
    api.updateQuietHours.mockImplementation(async (data) => ({ ...data, configured: true }));
    render(<QuietHoursSection />);
    fireEvent.click(await screen.findByRole('switch', { name: 'Quiet hours' }));
    fireEvent.change(screen.getByLabelText('to'), { target: { value: '07:30' } });
    fireEvent.click(screen.getByRole('switch', { name: 'Critical notifications always come through' }));
    expect(screen.getByTestId('quiet-hours-preview')).toHaveTextContent(
      'Quiet from 23:00 to 07:30 Europe/Amsterdam; held messages arrive as one summary at 07:30, critical notifications wait too.',
    );
    fireEvent.click(screen.getByRole('button', { name: 'Save quiet hours' }));
    await waitFor(() =>
      expect(api.updateQuietHours).toHaveBeenCalledWith({
        enabled: true,
        from_minute: 23 * 60,
        to_minute: 7 * 60 + 30,
        timezone: 'Europe/Amsterdam',
        critical_bypass: false,
      }),
    );
    expect(await screen.findByText('Quiet hours saved')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Save quiet hours' })).toBeDisabled();
  });

  it('blocks equal start and end times', async () => {
    api.getQuietHours.mockResolvedValue(quiet());
    render(<QuietHoursSection />);
    fireEvent.change(await screen.findByLabelText('to'), { target: { value: '23:00' } });
    expect(screen.getByText('Pick different start and end times.')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Save quiet hours' })).toBeDisabled();
  });

  it('shows a server error', async () => {
    api.getQuietHours.mockResolvedValue(quiet());
    api.updateQuietHours.mockRejectedValue({ response: { status: 400, data: { detail: 'Unknown time zone: Nowhere' } } });
    render(<QuietHoursSection />);
    fireEvent.click(await screen.findByRole('switch', { name: 'Quiet hours' }));
    fireEvent.click(screen.getByRole('button', { name: 'Save quiet hours' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('Unknown time zone: Nowhere');
  });

  it('shows a load error', async () => {
    api.getQuietHours.mockRejectedValue({ response: { status: 500, data: { detail: 'boom' } } });
    render(<QuietHoursSection />);
    expect(await screen.findByRole('alert')).toHaveTextContent('boom');
  });
});
