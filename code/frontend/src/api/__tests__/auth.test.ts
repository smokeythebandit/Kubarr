import { describe, it, expect, vi, beforeEach } from 'vitest';
import axios from 'axios';

const { mockPost } = vi.hoisted(() => ({
  mockPost: vi.fn(),
}));

vi.mock('axios', async () => {
  const actual = await vi.importActual<typeof import('axios')>('axios');
  return {
    ...actual,
    default: {
      ...actual.default,
      create: vi.fn(() => ({
        post: mockPost,
        get: vi.fn(),
        delete: vi.fn(),
        interceptors: {
          request: { use: vi.fn() },
          response: { use: vi.fn() },
        },
      })),
      isAxiosError: actual.default.isAxiosError,
    },
  };
});

import { sessionLogin } from '../auth';

describe('sessionLogin', () => {
  beforeEach(() => {
    mockPost.mockReset();
  });

  it('returns success with user info on successful login', async () => {
    mockPost.mockResolvedValue({
      data: { user_id: 1, username: 'admin', email: 'admin@test.com', session_slot: 0 },
    });

    const result = await sessionLogin({ username: 'admin', password: 'pass' });

    expect(result.status).toBe('success');
    expect(result.username).toBe('admin');
    expect(result.user_id).toBe(1);
    expect(mockPost).toHaveBeenCalledWith('/login', { username: 'admin', password: 'pass' });
  });

  it('returns 2fa_required when backend responds with 400 and 2FA detail', async () => {
    const error = new Error('Request failed') as Error & {
      response: { status: number; data: { detail: string } };
      isAxiosError: boolean;
    };
    error.response = {
      status: 400,
      data: { detail: 'Two-factor authentication code required' },
    };
    Object.defineProperty(error, 'isAxiosError', { value: true });

    mockPost.mockRejectedValue(error);
    vi.spyOn(axios, 'isAxiosError').mockReturnValue(true);

    const result = await sessionLogin({ username: 'user', password: 'pass' });

    expect(result.status).toBe('2fa_required');
  });

  it('throws on non-2FA errors', async () => {
    const error = new Error('Server error');
    (error as unknown as { response: { status: number; data: { detail: string } } }).response = {
      status: 500,
      data: { detail: 'Internal server error' },
    };

    mockPost.mockRejectedValue(error);
    vi.spyOn(axios, 'isAxiosError').mockReturnValue(false);

    await expect(sessionLogin({ username: 'user', password: 'pass' })).rejects.toThrow('Server error');
  });
});
