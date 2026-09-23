import { beforeEach, describe, expect, it, vi } from 'vitest';

const { mockGet } = vi.hoisted(() => ({ mockGet: vi.fn() }));
vi.mock('../client', () => ({ default: { get: mockGet } }));

import { securityApi, SECURITY_ACTIONS } from '../security';

beforeEach(() => mockGet.mockReset());

describe('security audit queries', () => {
  it('queries failed logins by the server action identifier', async () => {
    mockGet.mockResolvedValue({ data: { logs: [] } });
    await securityApi.getLoginFailures(10);
    expect(mockGet).toHaveBeenCalledWith('/audit?action=login_failed&per_page=10');
  });

  it('merges per-action results by time, without dropping events behind unrelated actions', async () => {
    mockGet.mockImplementation(async (url: string) => {
      const action = new URL(`http://local${url}`).searchParams.get('action');
      return { data: { logs: action === 'login_failed' ? [
        { id: 2, action, timestamp: '2026-01-02T00:00:00Z' },
      ] : action === 'login' ? [
        { id: 1, action, timestamp: '2026-01-01T00:00:00Z' },
      ] : [] } };
    });
    const result = await securityApi.getRecentSecurityEvents();
    expect(result.map(log => log.id)).toEqual([2, 1]);
    expect(mockGet).toHaveBeenCalledTimes(SECURITY_ACTIONS.length);
    for (const action of SECURITY_ACTIONS) {
      expect(mockGet).toHaveBeenCalledWith(`/audit?action=${encodeURIComponent(action)}&per_page=50`);
    }
    expect(mockGet).not.toHaveBeenCalledWith('/security/2fa/stats');
  });

  it('propagates failures instead of presenting an empty feed', async () => {
    mockGet.mockImplementation((url?: string) => url?.includes('action=login_failed')
      ? Promise.reject(new Error('Forbidden')) : Promise.resolve({ data: { logs: [] } }));
    await expect(securityApi.getRecentSecurityEvents()).rejects.toThrow('Forbidden');
  });
});
