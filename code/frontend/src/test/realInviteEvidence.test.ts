import { describe, expect, it, vi } from 'vitest';
import type { APIRequestContext } from '@playwright/test';
import { redeemInvite } from '../../tests/real/invite-redemption';

describe('real invitation evidence boundary', () => {
  it('keeps one-use code out of the request URL and returns only a status', async () => {
    const post = vi.fn().mockResolvedValue({ status: () => 200 });
    const code = 'one-use-fixture-code';
    expect(await redeemInvite({ post } as unknown as APIRequestContext, 'visitor', 'test-password', code)).toBe(200);
    expect(post).toHaveBeenCalledWith('/auth/register', {
      data: { username: 'visitor', email: 'visitor@example.test', password: 'test-password', invite_code: code },
    });
  });

  it('discards transport errors containing a one-use code before Playwright can report them', async () => {
    const code = 'one-use-fixture-code';
    const post = vi.fn().mockRejectedValue(new Error(`request with invite=${code} failed`));
    let message = '';
    try {
      await redeemInvite({ post } as unknown as APIRequestContext, 'visitor', 'test-password', code);
    } catch (error) {
      message = (error as Error).message;
    }
    expect(message).toBe('Invite redemption transport failed');
    expect(message).not.toContain(code);
  });
});
