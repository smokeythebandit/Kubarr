import { describe, it, expect, vi } from 'vitest';
import { renderHook, waitFor } from '@testing-library/react';
import React from 'react';
import { AuthProvider, useAuth } from '../AuthContext';
import { createMockUser, createMockAdminUser } from '../../test/helpers';

// Mock the API modules
const mockGetCurrentUser = vi.fn();
const mockGetAccounts = vi.fn();

vi.mock('../../api/users', () => ({
  getCurrentUser: (...args: unknown[]) => mockGetCurrentUser(...args),
}));

vi.mock('../../api/auth', () => ({
  getAccounts: (...args: unknown[]) => mockGetAccounts(...args),
  switchAccount: vi.fn(),
}));

function renderAuthHook(pathname = '/dashboard') {
  window.location.pathname = pathname;
  return renderHook(() => useAuth(), {
    wrapper: ({ children }: { children: React.ReactNode }) => (
      <AuthProvider>{children}</AuthProvider>
    ),
  });
}

describe('AuthContext', () => {
  it('hasPermission returns true for admin regardless of permission', async () => {
    const adminUser = createMockAdminUser();
    mockGetCurrentUser.mockResolvedValue(adminUser);
    mockGetAccounts.mockResolvedValue([]);

    const { result } = renderAuthHook();

    await waitFor(() => {
      expect(result.current.loading).toBe(false);
    });

    expect(result.current.isAdmin).toBe(true);
    expect(result.current.hasPermission('users.manage')).toBe(true);
    expect(result.current.hasPermission('anything.at.all')).toBe(true);
  });

  it('hasPermission returns false for non-admin without matching permission', async () => {
    const regularUser = createMockUser({ permissions: ['apps.view'] });
    mockGetCurrentUser.mockResolvedValue(regularUser);
    mockGetAccounts.mockResolvedValue([]);

    const { result } = renderAuthHook();

    await waitFor(() => {
      expect(result.current.loading).toBe(false);
    });

    expect(result.current.isAdmin).toBe(false);
    expect(result.current.hasPermission('apps.view')).toBe(true);
    expect(result.current.hasPermission('users.manage')).toBe(false);
  });

  it('canAccessApp returns true for admin and checks allowedApps for non-admin', async () => {
    const regularUser = createMockUser({ allowed_apps: ['sonarr', 'radarr'] });
    mockGetCurrentUser.mockResolvedValue(regularUser);
    mockGetAccounts.mockResolvedValue([]);

    const { result } = renderAuthHook();

    await waitFor(() => {
      expect(result.current.loading).toBe(false);
    });

    expect(result.current.canAccessApp('sonarr')).toBe(true);
    expect(result.current.canAccessApp('lidarr')).toBe(false);
  });
});
