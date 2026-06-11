import apiClient from './client';
import { RoleInfo } from './roles';

export type Theme = 'system' | 'light' | 'dark';

export interface UserPreferences {
  theme: Theme;
}

export interface User {
  id: number;
  username: string;
  email: string;
  is_active: boolean;
  is_approved: boolean;
  created_at: string;
  updated_at: string;
  roles: RoleInfo[];
  preferences: UserPreferences;
  permissions: string[];
  allowed_apps: string[];
}

export interface CreateUserRequest {
  username: string;
  email: string;
  password: string;
  role_ids?: number[];
}

export interface UpdateUserRequest {
  email?: string;
  is_active?: boolean;
  is_approved?: boolean;
  role_ids?: number[];
}

/**
 * Get current authenticated user
 */
export const getCurrentUser = async (): Promise<User> => {
  const response = await apiClient.get<User>('/users/me');
  return response.data;
};

/**
 * Get all users (admin only)
 */
export const getUsers = async (): Promise<User[]> => {
  const response = await apiClient.get<User[]>('/users');
  return response.data;
};

/**
 * Get pending approval users (admin only)
 */
export const getPendingUsers = async (): Promise<User[]> => {
  const response = await apiClient.get<User[]>('/users/pending');
  return response.data;
};

/**
 * Get user by ID (admin only)
 */
export const getUser = async (userId: number): Promise<User> => {
  const response = await apiClient.get<User>(`/users/${userId}`);
  return response.data;
};

/**
 * Create a new user (admin only)
 */
export const createUser = async (userData: CreateUserRequest): Promise<User> => {
  const response = await apiClient.post<User>('/users', userData);
  return response.data;
};

/**
 * Update user (admin only)
 */
export const updateUser = async (userId: number, userData: UpdateUserRequest): Promise<User> => {
  const response = await apiClient.patch<User>(`/users/${userId}`, userData);
  return response.data;
};

/**
 * Approve user registration (admin only)
 */
export const approveUser = async (userId: number): Promise<{ message: string }> => {
  const response = await apiClient.post<{ message: string }>(`/users/${userId}/approve`);
  return response.data;
};

/**
 * Reject user registration (admin only)
 */
export const rejectUser = async (userId: number): Promise<{ message: string }> => {
  const response = await apiClient.post<{ message: string }>(`/users/${userId}/reject`);
  return response.data;
};

/**
 * Delete user (admin only)
 */
export const deleteUser = async (userId: number): Promise<{ message: string }> => {
  const response = await apiClient.delete<{ message: string }>(`/users/${userId}`);
  return response.data;
};

// Invite types and API functions
export interface Invite {
  id: number;
  code: string;
  created_by_username: string;
  used_by_username: string | null;
  is_used: boolean;
  expires_at: string | null;
  created_at: string;
  used_at: string | null;
}

export interface CreateInviteRequest {
  expires_in_days?: number;
}

/**
 * Get all invites (admin only)
 */
export const getInvites = async (): Promise<Invite[]> => {
  const response = await apiClient.get<Invite[]>('/users/invites');
  return response.data;
};

/**
 * Create a new invite (admin only)
 */
export const createInvite = async (data?: CreateInviteRequest): Promise<Invite> => {
  const response = await apiClient.post<Invite>('/users/invites', data || {});
  return response.data;
};

/**
 * Delete an invite (admin only)
 */
export const deleteInvite = async (inviteId: number): Promise<{ message: string }> => {
  const response = await apiClient.delete<{ message: string }>(`/users/invites/${inviteId}`);
  return response.data;
};

// Update own profile API

export interface UpdateOwnProfileRequest {
  username?: string;
  email?: string;
}

/**
 * Update current user's own profile (username, email)
 */
export const updateOwnProfile = async (data: UpdateOwnProfileRequest): Promise<User> => {
  const response = await apiClient.patch<User>('/users/me', data);
  return response.data;
};

// Preferences API functions

export interface UpdatePreferencesRequest {
  theme?: Theme;
}

/**
 * Get current user's preferences
 */
export const getMyPreferences = async (): Promise<UserPreferences> => {
  const response = await apiClient.get<UserPreferences>('/users/me/preferences');
  return response.data;
};

/**
 * Update current user's preferences
 */
export const updateMyPreferences = async (data: UpdatePreferencesRequest): Promise<UserPreferences> => {
  const response = await apiClient.patch<UserPreferences>('/users/me/preferences', data);
  return response.data;
};

// ============================================================================
// Password Change API
// ============================================================================

export interface ChangeOwnPasswordRequest {
  current_password: string;
  new_password: string;
}

export interface AdminResetPasswordRequest {
  new_password: string;
}

/**
 * Change own password (requires current password)
 */
export const changeOwnPassword = async (data: ChangeOwnPasswordRequest): Promise<{ message: string }> => {
  const response = await apiClient.patch<{ message: string }>('/users/me/password', data);
  return response.data;
};

/**
 * Admin reset password for another user (requires users.manage permission)
 */
export const adminResetPassword = async (userId: number, data: AdminResetPasswordRequest): Promise<{ message: string }> => {
  const response = await apiClient.patch<{ message: string }>(`/users/${userId}/password`, data);
  return response.data;
};

// ============================================================================
// Two-Factor Authentication API
// ============================================================================

export interface TwoFactorSetupResponse {
  secret: string;
  provisioning_uri: string;
}

export interface TwoFactorStatusResponse {
  enabled: boolean;
  verified_at: string | null;
  required_by_role: boolean;
}

/**
 * Set up 2FA - generates secret and QR code
 */
export const setup2FA = async (): Promise<TwoFactorSetupResponse> => {
  const response = await apiClient.post<TwoFactorSetupResponse>('/users/me/2fa/setup');
  return response.data;
};

export interface TwoFactorEnableResponse {
  message: string;
  recovery_codes: string[];
}

export interface TwoFactorRecoveryCodesResponse {
  remaining: number;
}

/**
 * Enable 2FA - verifies code and activates; returns one-time recovery codes
 */
export const enable2FA = async (code: string): Promise<TwoFactorEnableResponse> => {
  const response = await apiClient.post<TwoFactorEnableResponse>('/users/me/2fa/enable', { code });
  return response.data;
};

/**
 * Get remaining unused recovery code count
 */
export const getRecoveryCodeCount = async (): Promise<TwoFactorRecoveryCodesResponse> => {
  const response = await apiClient.get<TwoFactorRecoveryCodesResponse>('/users/me/2fa/recovery-codes');
  return response.data;
};

/**
 * Disable 2FA (requires password confirmation)
 */
export const disable2FA = async (password: string): Promise<{ message: string }> => {
  const response = await apiClient.post<{ message: string }>('/users/me/2fa/disable', { password });
  return response.data;
};

/**
 * Get 2FA status
 */
export const get2FAStatus = async (): Promise<TwoFactorStatusResponse> => {
  const response = await apiClient.get<TwoFactorStatusResponse>('/users/me/2fa/status');
  return response.data;
};

// ============================================================================
// Delete Own Account API
// ============================================================================

export interface DeleteOwnAccountRequest {
  password: string;
}

/**
 * Delete own account (requires password confirmation)
 */
export const deleteOwnAccount = async (data: DeleteOwnAccountRequest): Promise<{ message: string }> => {
  const response = await apiClient.delete<{ message: string }>('/users/me', { data });
  return response.data;
};
