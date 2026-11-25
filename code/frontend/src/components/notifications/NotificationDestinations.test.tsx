import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, fireEvent, render, screen } from '@testing-library/react';
import type { UserNotificationPref } from '../../api/notifications';
import NotificationDestinations, { DestinationRow } from './NotificationDestinations';
import { retryAfterSeconds } from './verificationErrors';

vi.mock('../../api/notifications', () => ({
  notificationsApi: {
    getPreferences: vi.fn(),
    updatePreference: vi.fn(),
    startVerification: vi.fn(),
    confirmVerification: vi.fn(),
  },
}));

import { notificationsApi } from '../../api/notifications';

const api = vi.mocked(notificationsApi);

const pref = (overrides: Partial<UserNotificationPref> = {}): UserNotificationPref => ({
  channel_type: 'telegram',
  enabled: true,
  destination: '123***89',
  verified: false,
  verification_pending: false,
  ...overrides,
});

const httpError = (status: number, detail: string, headers: Record<string, string> = {}) => ({
  response: { status, data: { detail }, headers },
});

/** Let pending promises (mocked API calls) settle. */
async function flush() {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
}

beforeEach(() => {
  vi.useFakeTimers();
  vi.clearAllMocks();
});

afterEach(() => {
  vi.useRealTimers();
});

describe('DestinationRow', () => {
  it('sends a code, counts down, and confirms to Verified', async () => {
    api.startVerification.mockResolvedValue({ destination: '123***89', expires_in_seconds: 600, resend_after_seconds: 60 });
    api.confirmVerification.mockResolvedValue(pref({ verified: true }));
    render(<DestinationRow initial={pref()} />);

    expect(screen.getByText('Not verified')).toBeInTheDocument();
    expect(screen.getByText('Kubarr only sends here after you confirm it.')).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: 'Send code' }));
    await flush();
    expect(api.startVerification).toHaveBeenCalledWith('telegram');
    expect(screen.getByRole('status')).toHaveTextContent('Code sent to 123***89');
    expect(screen.getByText('Code sent')).toBeInTheDocument();

    const resend = screen.getByRole('button', { name: /Send code \(60s\)/ });
    expect(resend).toBeDisabled();
    await act(async () => {
      vi.advanceTimersByTime(1000);
    });
    expect(screen.getByRole('button', { name: /\(59s\)/ })).toBeDisabled();

    const input = screen.getByLabelText('Verification code');
    expect(input).toHaveAttribute('autocomplete', 'one-time-code');
    fireEvent.change(input, { target: { value: '424242' } });
    fireEvent.click(screen.getByRole('button', { name: 'Confirm' }));
    await flush();

    expect(api.confirmVerification).toHaveBeenCalledWith('telegram', '424242');
    expect(screen.getByText('Verified')).toBeInTheDocument();
    expect(screen.queryByLabelText('Verification code')).not.toBeInTheDocument();
    expect(screen.queryByText('Kubarr only sends here after you confirm it.')).not.toBeInTheDocument();
  });

  it('starts the countdown from Retry-After on 429', async () => {
    api.startVerification.mockRejectedValue(
      httpError(429, 'Please wait 42 seconds before requesting another verification code', { 'retry-after': '42' }),
    );
    render(<DestinationRow initial={pref()} />);

    fireEvent.click(screen.getByRole('button', { name: 'Send code' }));
    await flush();

    expect(screen.getByRole('alert')).toHaveTextContent('Please wait 42 seconds');
    expect(screen.getByRole('button', { name: /Send code \(42s\)/ })).toBeDisabled();
    for (let i = 0; i < 42; i++) {
      await act(async () => {
        vi.advanceTimersByTime(1000);
      });
    }
    expect(screen.getByRole('button', { name: 'Send code' })).toBeEnabled();
  });

  it('shows the server message with attempts left on a wrong code', async () => {
    api.confirmVerification.mockRejectedValue(httpError(400, 'Incorrect verification code; 4 attempts left'));
    render(<DestinationRow initial={pref({ verification_pending: true })} />);

    fireEvent.change(screen.getByLabelText('Verification code'), { target: { value: '000000' } });
    fireEvent.click(screen.getByRole('button', { name: 'Confirm' }));
    await flush();

    expect(screen.getByRole('alert')).toHaveTextContent('4 attempts left');
    expect(screen.getByText('Code sent')).toBeInTheDocument();
  });

  it('resets verification when the destination changes', async () => {
    api.updatePreference.mockResolvedValue(pref({ destination: '987***21', verified: false, verification_pending: false }));
    render(<DestinationRow initial={pref({ verified: false, verification_pending: true })} />);
    expect(screen.getByLabelText('Verification code')).toBeInTheDocument();

    fireEvent.change(screen.getByLabelText('Chat ID'), { target: { value: '987654321' } });
    fireEvent.click(screen.getByRole('button', { name: 'Save' }));
    await flush();

    expect(api.updatePreference).toHaveBeenCalledWith('telegram', { enabled: true, destination: '987654321' });
    expect(screen.getByText('Not verified')).toBeInTheDocument();
    expect(screen.queryByLabelText('Verification code')).not.toBeInTheDocument();
    expect(screen.getByLabelText('Chat ID')).toHaveAttribute('placeholder', 'Current: 987***21');
  });

  it('toggles the channel with the switch', async () => {
    api.updatePreference.mockResolvedValue(pref({ enabled: false }));
    render(<DestinationRow initial={pref()} />);
    const toggle = screen.getByRole('switch', { name: 'Telegram notifications' });
    expect(toggle).toHaveAttribute('aria-checked', 'true');
    fireEvent.click(toggle);
    await flush();
    expect(api.updatePreference).toHaveBeenCalledWith('telegram', { enabled: false });
    expect(toggle).toHaveAttribute('aria-checked', 'false');
    expect(screen.getByRole('button', { name: 'Send code' })).toBeDisabled();
  });
});

describe('NotificationDestinations', () => {
  it('renders one row per channel', async () => {
    api.getPreferences.mockResolvedValue([
      pref({ channel_type: 'email', destination: 'al***@example.com', verified: true }),
      pref(),
      pref({ channel_type: 'messagebird', enabled: false, destination: null }),
    ]);
    render(<NotificationDestinations />);
    await flush();
    expect(screen.getByText('Email')).toBeInTheDocument();
    expect(screen.getByText('SMS')).toBeInTheDocument();
    expect(screen.getByText('Verified')).toBeInTheDocument();
    expect(screen.getByText('Not verified')).toBeInTheDocument();
  });
});

describe('retryAfterSeconds', () => {
  it('reads the header only on 429', () => {
    expect(retryAfterSeconds(httpError(429, 'x', { 'retry-after': '17' }))).toBe(17);
    expect(retryAfterSeconds(httpError(429, 'x'))).toBeNull();
    expect(retryAfterSeconds(httpError(400, 'x', { 'retry-after': '17' }))).toBeNull();
  });
});
