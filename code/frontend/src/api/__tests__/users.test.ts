import { describe, it, expect, vi } from 'vitest';

const { mockGet, mockDelete } = vi.hoisted(() => ({
  mockGet: vi.fn(),
  mockDelete: vi.fn(),
}));

vi.mock('../client', () => ({
  default: {
    get: mockGet,
    delete: mockDelete,
    post: vi.fn(),
    patch: vi.fn(),
  },
}));

import { getCurrentUser, deleteUser } from '../users';

describe('getCurrentUser', () => {
  it('returns mapped user data from /users/me', async () => {
    const mockUser = {
      id: 1,
      username: 'testuser',
      email: 'test@example.com',
      is_active: true,
      is_approved: true,
      created_at: '2024-01-01T00:00:00Z',
      updated_at: '2024-01-01T00:00:00Z',
      roles: [{ id: 1, name: 'admin', description: 'Admin' }],
      preferences: { theme: 'dark' },
      permissions: ['apps.view'],
      allowed_apps: ['sonarr'],
    };
    mockGet.mockResolvedValue({ data: mockUser });

    const result = await getCurrentUser();

    expect(mockGet).toHaveBeenCalledWith('/users/me');
    expect(result).toEqual(mockUser);
    expect(result.username).toBe('testuser');
    expect(result.permissions).toContain('apps.view');
  });
});

describe('deleteUser', () => {
  it('sends DELETE to /users/:id', async () => {
    mockDelete.mockResolvedValue({ data: { message: 'User deleted' } });

    const result = await deleteUser(42);

    expect(mockDelete).toHaveBeenCalledWith('/users/42');
    expect(result.message).toBe('User deleted');
  });
});
