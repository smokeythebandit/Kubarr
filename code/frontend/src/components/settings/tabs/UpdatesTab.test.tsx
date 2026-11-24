import { beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { UpdatesTab } from './UpdatesTab';

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
  window_start: '04:00',
  window_minutes: 120,
  timezone: 'UTC',
  health_timeout_minutes: 10,
  next_window_start: null,
};

describe('UpdatesTab', () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    mocks.getSettings.mockReset().mockResolvedValue(defaults);
    mocks.saveSettings.mockReset().mockImplementation(async (s) => ({ ...s, next_window_start: null }));
    mocks.canManage = true;
    vi.spyOn(Intl, 'DateTimeFormat').mockImplementation(
      () => ({ resolvedOptions: () => ({ timeZone: 'Europe/Amsterdam' }) }) as unknown as Intl.DateTimeFormat,
    );
  });

  it('renders the server values', async () => {
    mocks.getSettings.mockResolvedValue({ ...defaults, timezone: 'America/New_York', window_start: '23:00', window_minutes: 180 });
    render(<UpdatesTab />);
    expect(await screen.findByLabelText('Window start')).toHaveValue('23:00');
    expect(screen.getByLabelText('Window length (minutes)')).toHaveValue(180);
    expect(screen.getByLabelText('Timezone')).toHaveValue('America/New_York');
    expect(screen.getByLabelText('Health check timeout (minutes)')).toHaveValue(10);
    expect(screen.getByRole('switch', { name: 'Automatic updates' })).toHaveAttribute('aria-checked', 'true');
  });

  it('preselects the browser timezone when the server still reports UTC', async () => {
    render(<UpdatesTab />);
    expect(await screen.findByLabelText('Timezone')).toHaveValue('Europe/Amsterdam');
  });

  it('saves the edited window and timezone', async () => {
    render(<UpdatesTab />);
    fireEvent.change(await screen.findByLabelText('Window start'), { target: { value: '02:30' } });
    fireEvent.change(screen.getByLabelText('Window length (minutes)'), { target: { value: '90' } });
    fireEvent.change(screen.getByLabelText('Timezone'), { target: { value: 'Europe/Berlin' } });
    fireEvent.click(screen.getByRole('switch', { name: 'Automatic updates' }));
    fireEvent.click(screen.getByRole('button', { name: 'Save' }));
    await waitFor(() => expect(mocks.saveSettings).toHaveBeenCalledWith({
      enabled: false,
      window_start: '02:30',
      window_minutes: 90,
      timezone: 'Europe/Berlin',
      health_timeout_minutes: 10,
    }));
    expect(await screen.findByText('Update settings saved.')).toBeVisible();
  });

  it("shows the server's 400 message", async () => {
    mocks.saveSettings.mockRejectedValue({ response: { data: { detail: 'window_minutes: must be between 30 and 720' } } });
    render(<UpdatesTab />);
    await screen.findByLabelText('Window start');
    fireEvent.click(screen.getByRole('button', { name: 'Save' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('window_minutes: must be between 30 and 720');
  });

  it('is read-only without settings.manage', async () => {
    mocks.canManage = false;
    render(<UpdatesTab />);
    expect(await screen.findByLabelText('Window start')).toBeDisabled();
    expect(screen.queryByRole('button', { name: 'Save' })).not.toBeInTheDocument();
  });
});
