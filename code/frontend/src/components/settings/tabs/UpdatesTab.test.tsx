import { beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { UpdatesTab } from './UpdatesTab';
import { emptySchedule, presetSchedule } from '../weekSchedule';

const mocks = vi.hoisted(() => ({
  getSettings: vi.fn(),
  saveSettings: vi.fn(),
  canManage: true,
}));

vi.mock('../../../api/updates', () => ({
  updatesApi: {
    getSettings: (...a: unknown[]) => mocks.getSettings(...a),
    saveSettings: (...a: unknown[]) => mocks.saveSettings(...a),
  },
}));

vi.mock('../../../contexts/AuthContext', () => ({
  useAuth: () => ({ hasPermission: (p: string) => p === 'settings.manage' && mocks.canManage }),
}));

const defaults = {
  enabled: true,
  schedule: presetSchedule('nightly'),
  timezone: 'UTC',
  health_timeout_minutes: 10,
  next_window_start: null,
  timezone_saved: false,
};

describe('UpdatesTab', () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    mocks.getSettings.mockReset().mockResolvedValue(defaults);
    mocks.saveSettings.mockReset().mockImplementation(async (s) => ({ ...s, next_window_start: null, timezone_saved: true }));
    mocks.canManage = true;
    const real = Intl.DateTimeFormat.prototype.resolvedOptions;
    vi.spyOn(Intl.DateTimeFormat.prototype, 'resolvedOptions').mockImplementation(function (this: Intl.DateTimeFormat) {
      return { ...real.call(this), timeZone: 'Europe/Amsterdam' };
    });
  });

  it('renders the server values', async () => {
    const schedule = emptySchedule();
    schedule[0][23] = true;
    mocks.getSettings.mockResolvedValue({ ...defaults, timezone: 'America/New_York', timezone_saved: true, schedule });
    render(<UpdatesTab />);
    expect(await screen.findByRole('gridcell', { name: 'Mon 23:00' })).toHaveAttribute('aria-selected', 'true');
    expect(screen.getByRole('gridcell', { name: 'Mon 04:00' })).toHaveAttribute('aria-selected', 'false');
    expect(screen.getByLabelText('Timezone')).toHaveValue('America/New_York');
    expect(screen.getByLabelText('Health check timeout (minutes)')).toHaveValue(10);
    // The worker needs time to become healthy plus a 3-minute settle period.
    expect(screen.getByLabelText('Health check timeout (minutes)')).toHaveAttribute('min', '5');
    expect(screen.getByLabelText('Health check timeout (minutes)')).toHaveAttribute('max', '60');
    expect(screen.getByRole('switch', { name: 'Automatic updates' })).toHaveAttribute('aria-checked', 'true');
  });

  it('preselects the browser timezone and hints while no zone is saved', async () => {
    render(<UpdatesTab />);
    expect(await screen.findByLabelText('Timezone')).toHaveValue('Europe/Amsterdam');
    expect(screen.getByText('Not saved yet — Kubarr uses UTC until you save.')).toBeVisible();
  });

  it('keeps a deliberately saved UTC', async () => {
    mocks.getSettings.mockResolvedValue({ ...defaults, timezone_saved: true });
    render(<UpdatesTab />);
    expect(await screen.findByLabelText('Timezone')).toHaveValue('UTC');
    expect(screen.queryByText(/Not saved yet/)).not.toBeInTheDocument();
  });

  it('saves the edited schedule and timezone', async () => {
    render(<UpdatesTab />);
    fireEvent.click(await screen.findByRole('button', { name: 'Weekdays 10–16' }));
    fireEvent.change(screen.getByLabelText('Timezone'), { target: { value: 'Europe/Berlin' } });
    fireEvent.click(screen.getByRole('switch', { name: 'Automatic updates' }));
    fireEvent.click(screen.getByRole('button', { name: 'Save' }));
    await waitFor(() => expect(mocks.saveSettings).toHaveBeenCalledWith({
      enabled: false,
      schedule: presetSchedule('workday'),
      timezone: 'Europe/Berlin',
      health_timeout_minutes: 10,
    }));
    expect(await screen.findByText('Update settings saved.')).toBeVisible();
  });

  it('warns when enabled with no hours selected', async () => {
    render(<UpdatesTab />);
    fireEvent.click(await screen.findByRole('button', { name: 'Clear' }));
    expect(screen.getByText("No hours selected — automatic updates won't run.")).toBeVisible();
    fireEvent.click(screen.getByRole('switch', { name: 'Automatic updates' }));
    expect(screen.queryByText(/automatic updates won't run/)).not.toBeInTheDocument();
  });

  it("shows the server's 400 message", async () => {
    mocks.saveSettings.mockRejectedValue({ response: { data: { detail: 'schedule: expected 7 days of 24 hours each' } } });
    render(<UpdatesTab />);
    fireEvent.click(await screen.findByRole('button', { name: 'Save' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('schedule: expected 7 days of 24 hours each');
  });

  it('is read-only without settings.manage', async () => {
    mocks.canManage = false;
    render(<UpdatesTab />);
    expect(await screen.findByRole('button', { name: 'Clear' })).toBeDisabled();
    expect(screen.getByLabelText('Timezone')).toBeDisabled();
    fireEvent.keyDown(screen.getByRole('gridcell', { name: 'Mon 10:00' }), { key: ' ' });
    expect(screen.getByRole('gridcell', { name: 'Mon 10:00' })).toHaveAttribute('aria-selected', 'false');
    expect(screen.queryByRole('button', { name: 'Save' })).not.toBeInTheDocument();
  });
});
