import { describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen } from '@testing-library/react';
import { GeneralTab } from './GeneralTab';
import type { Setting } from '../../../api/settings';

function setting(key: string, value: string): Setting {
  return { key, value };
}

function props(overrides: Partial<Parameters<typeof GeneralTab>[0]> = {}) {
  return {
    systemSettings: {} as Record<string, Setting>,
    settingsStatus: 'ready' as const,
    savingSettings: false,
    oauthProviders: [],
    editingOAuthProvider: null,
    oauthProviderConfig: { client_id: '', client_secret: '' },
    savingOAuthProvider: false,
    onToggleSetting: vi.fn(),
    onToggleOAuthProvider: vi.fn(),
    onSaveOAuthProvider: vi.fn(),
    setEditingOAuthProvider: vi.fn(),
    setOauthProviderConfig: vi.fn(),
    getOAuthProviderIcon: () => null,
    ...overrides,
  };
}

const registration = () => screen.getByRole('switch', { name: 'Allow Open Registration' });

describe('GeneralTab registration switches', () => {
  it('reflects a loaded value', () => {
    render(<GeneralTab {...props({
      systemSettings: { registration_enabled: setting('registration_enabled', 'true') },
    })} />);
    expect(registration().getAttribute('aria-checked')).toBe('true');
    expect((registration() as HTMLButtonElement).disabled).toBe(false);
  });

  // The bug that blocked the 0.3.10 release: a settings fetch that failed left
  // systemSettings as {}, and `value === 'true'` rendered that as a confident
  // "off" -- indistinguishable from the setting genuinely being off. The
  // acceptance test flipped the setting, saw the API agree, reloaded, and then
  // read "off" back out of a page that had simply never loaded anything.
  it('does not claim a setting is off when the fetch failed', () => {
    render(<GeneralTab {...props({ settingsStatus: 'error' })} />);
    expect(registration().getAttribute('aria-checked')).toBe('mixed');
    expect((registration() as HTMLButtonElement).disabled).toBe(true);
    expect(screen.getByText(/could not be loaded/i)).toBeTruthy();
  });

  it('does not claim a setting is off while still loading', () => {
    render(<GeneralTab {...props({ settingsStatus: 'loading' })} />);
    expect(registration().getAttribute('aria-checked')).toBe('mixed');
    expect((registration() as HTMLButtonElement).disabled).toBe(true);
  });

  it('ignores clicks until the real value is known', () => {
    const onToggleSetting = vi.fn();
    render(<GeneralTab {...props({ settingsStatus: 'error', onToggleSetting })} />);
    fireEvent.click(registration());
    expect(onToggleSetting).not.toHaveBeenCalled();
  });

  it('still blocks interaction while a save is in flight', () => {
    render(<GeneralTab {...props({
      systemSettings: { registration_enabled: setting('registration_enabled', 'true') },
      savingSettings: true,
    })} />);
    expect((registration() as HTMLButtonElement).disabled).toBe(true);
    expect(registration().getAttribute('aria-checked')).toBe('true');
  });
});
