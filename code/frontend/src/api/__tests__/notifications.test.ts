import { describe, it, expect, vi } from 'vitest';

const { mockGet, mockPut } = vi.hoisted(() => ({
  mockGet: vi.fn(),
  mockPut: vi.fn(),
}));

vi.mock('../client', () => ({
  default: {
    get: mockGet,
    put: mockPut,
    post: vi.fn(),
    delete: vi.fn(),
  },
}));

import { notificationsApi } from '../notifications';

describe('notification routes', () => {
  it('lists routes from /notifications/routes', async () => {
    const routes = [{ event_type: 'login_failed', channels: { telegram: true, email: false, messagebird: false } }];
    mockGet.mockResolvedValue({ data: routes });
    await expect(notificationsApi.getRoutes()).resolves.toEqual(routes);
    expect(mockGet).toHaveBeenCalledWith('/notifications/routes');
  });

  it('puts only the changed channel', async () => {
    const updated = { event_type: 'login_failed', channels: { telegram: false, email: true, messagebird: false } };
    mockPut.mockResolvedValue({ data: updated });
    await expect(notificationsApi.updateRoute('login_failed', { telegram: false })).resolves.toEqual(updated);
    expect(mockPut).toHaveBeenCalledWith('/notifications/routes/login_failed', { channels: { telegram: false } });
  });
});
