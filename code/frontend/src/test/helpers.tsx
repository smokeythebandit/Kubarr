import React from 'react';
import { render, RenderOptions } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { User } from '../api/users';

export function createMockUser(overrides: Partial<User> = {}): User {
  return {
    id: 1,
    username: 'testuser',
    email: 'test@example.com',
    is_active: true,
    is_approved: true,
    created_at: '2024-01-01T00:00:00Z',
    updated_at: '2024-01-01T00:00:00Z',
    roles: [{ id: 2, name: 'user', description: 'Regular user' }],
    preferences: { theme: 'system' },
    permissions: ['apps.view'],
    allowed_apps: ['sonarr', 'radarr'],
    ...overrides,
  };
}

export function createMockAdminUser(overrides: Partial<User> = {}): User {
  return createMockUser({
    id: 99,
    username: 'admin',
    email: 'admin@example.com',
    roles: [{ id: 1, name: 'admin', description: 'Administrator' }],
    permissions: ['*'],
    allowed_apps: [],
    ...overrides,
  });
}

export function renderWithRouter(
  ui: React.ReactElement,
  { initialEntries = ['/'], ...options }: RenderOptions & { initialEntries?: string[] } = {},
) {
  return render(ui, {
    wrapper: ({ children }) => (
      <MemoryRouter initialEntries={initialEntries}>{children}</MemoryRouter>
    ),
    ...options,
  });
}
