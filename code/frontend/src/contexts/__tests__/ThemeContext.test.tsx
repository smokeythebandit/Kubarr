import { describe, it, expect, vi } from 'vitest';
import { renderHook, act, waitFor } from '@testing-library/react';
import React from 'react';
import { ThemeProvider, useTheme } from '../ThemeContext';
import { AuthProvider } from '../AuthContext';

// Mock the API modules
vi.mock('../../api/users', () => ({
  getCurrentUser: vi.fn().mockRejectedValue(new Error('Not authenticated')),
  updateMyPreferences: vi.fn().mockResolvedValue({ theme: 'light' }),
}));

vi.mock('../../api/auth', () => ({
  getAccounts: vi.fn().mockResolvedValue([]),
  switchAccount: vi.fn(),
}));

function renderThemeHook() {
  // ThemeProvider depends on AuthProvider
  return renderHook(() => useTheme(), {
    wrapper: ({ children }: { children: React.ReactNode }) => (
      <AuthProvider>
        <ThemeProvider>{children}</ThemeProvider>
      </AuthProvider>
    ),
  });
}

describe('ThemeContext', () => {
  it('resolves "light" theme correctly', async () => {
    window.location.pathname = '/login'; // skip auth check
    localStorage.setItem('kubarr-theme', 'light');

    const { result } = renderThemeHook();

    await waitFor(() => {
      expect(result.current.resolvedTheme).toBe('light');
    });

    expect(result.current.theme).toBe('light');
  });

  it('setTheme updates localStorage', async () => {
    window.location.pathname = '/login'; // skip auth check
    localStorage.removeItem('kubarr-theme');

    const { result } = renderThemeHook();

    await act(async () => {
      result.current.setTheme('dark');
    });

    expect(localStorage.setItem).toHaveBeenCalledWith('kubarr-theme', 'dark');
    expect(result.current.theme).toBe('dark');
    expect(result.current.resolvedTheme).toBe('dark');
  });
});
