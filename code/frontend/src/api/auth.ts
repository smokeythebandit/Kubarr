import axios from 'axios';

// Auth API uses a separate client without /api prefix since auth routes are at /auth/*
const authClient = axios.create({
  baseURL: '/auth',
  withCredentials: true,
});

// ============================================================================
// Types
// ============================================================================

export type SessionLoginStatus = 'success' | '2fa_required' | '2fa_setup_required';

export interface SessionLoginResponse {
  status: SessionLoginStatus;
  challenge_token?: string;
  // Backend returns these on success
  user_id?: number;
  username?: string;
  email?: string;
  session_slot?: number;
}

export interface AccountInfo {
  slot: number;
  user_id: number;
  username: string;
  email: string;
  is_active: boolean;
}

export interface LoginRequest {
  username: string;
  password: string;
  totp_code?: string;
}

export interface Verify2FARequest {
  challenge_token: string;
  code: string;
}

export interface SessionInfo {
  id: string;
  user_agent: string | null;
  ip_address: string | null;
  created_at: string;
  last_accessed_at: string;
  is_current: boolean;
}

// ============================================================================
// Session Auth API
// ============================================================================

/**
 * Login with username and password
 * Returns user info on success, or requires 2FA
 */
export const sessionLogin = async (credentials: LoginRequest): Promise<SessionLoginResponse> => {
  try {
    const response = await authClient.post<SessionLoginResponse>('/login', credentials);
    // Backend returns user info directly on success
    return { ...response.data, status: 'success' };
  } catch (error: unknown) {
    if (axios.isAxiosError(error) && error.response?.status === 400) {
      const detail = error.response.data?.detail || '';
      if (detail.includes('Two-factor authentication code required')) {
        return { status: '2fa_required' };
      }
      if (detail.includes('Two-factor authentication setup required')) {
        return { status: '2fa_setup_required' };
      }
    }
    throw error;
  }
};

/**
 * Verify 2FA code during login (sends credentials again with TOTP)
 */
export const verify2FA = async (
  credentials: LoginRequest,
  code: string
): Promise<SessionLoginResponse> => {
  return sessionLogin({ ...credentials, totp_code: code });
};

/**
 * Logout (clears session cookie)
 */
export const sessionLogout = async (): Promise<{ success: boolean }> => {
  const response = await authClient.post('/logout');
  return { success: true, ...response.data };
};

/**
 * Verify current session is valid by checking /api/users/me
 */
export const verifySession = async (): Promise<boolean> => {
  try {
    // Use the main API client to check if authenticated
    const response = await axios.get('/api/users/me', { withCredentials: true });
    return response.status === 200;
  } catch {
    return false;
  }
};

/**
 * Get all active sessions for the current user
 */
export const getSessions = async (): Promise<SessionInfo[]> => {
  const response = await authClient.get<SessionInfo[]>('/sessions');
  return response.data;
};

/**
 * Revoke a specific session
 */
export const revokeSession = async (sessionId: string): Promise<void> => {
  await authClient.delete(`/sessions/${sessionId}`);
};

/**
 * Get all signed-in accounts
 */
export const getAccounts = async (): Promise<AccountInfo[]> => {
  const response = await authClient.get<AccountInfo[]>('/accounts');
  return response.data;
};

/**
 * Switch to a different account by slot
 */
export const switchAccount = async (slot: number): Promise<void> => {
  await authClient.post(`/switch/${slot}`);
};

/**
 * Login using a recovery code instead of TOTP
 */
export const loginWithRecoveryCode = async (
  username: string,
  password: string,
  recoveryCode: string
): Promise<SessionLoginResponse> => {
  const response = await authClient.post<SessionLoginResponse>('/2fa/recover', {
    username,
    password,
    recovery_code: recoveryCode,
  });
  return { ...response.data, status: 'success' };
};
