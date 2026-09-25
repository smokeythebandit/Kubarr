import type { APIRequestContext } from '@playwright/test';

// This deliberately uses a fixed API path rather than navigating to a public
// invitation URL. Only return the status; never propagate a transport error
// that might embed a request body or token in native Playwright reports.
export async function redeemInvite(request: APIRequestContext, username: string, password: string, code: string): Promise<number> {
  try {
    const response = await request.post('/auth/register', {
      data: { username, email: `${username}@example.test`, password, invite_code: code },
    });
    return response.status();
  } catch {
    throw new Error('Invite redemption transport failed');
  }
}
