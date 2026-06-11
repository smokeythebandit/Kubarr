import apiClient from './client';

export interface OAuthProvider {
  id: string;
  name: string;
  enabled: boolean;
  client_id: string | null;
  has_secret: boolean;
}

export interface AvailableProvider {
  id: string;
  name: string;
}

export interface LinkedAccount {
  provider: string;
  email: string | null;
  display_name: string | null;
  linked_at: string;
}

export const oauthApi = {
  // Get available (enabled) OAuth providers - public endpoint for login page
  getAvailableProviders: async (): Promise<AvailableProvider[]> => {
    const response = await apiClient.get<AvailableProvider[]>('/oauth/available');
    return response.data;
  },

  // Get list of all OAuth providers (admin)
  getProviders: async (): Promise<OAuthProvider[]> => {
    const response = await apiClient.get<OAuthProvider[]>('/oauth/providers');
    return response.data;
  },

  // Get a specific provider's config (admin)
  getProvider: async (provider: string): Promise<OAuthProvider> => {
    const response = await apiClient.get<OAuthProvider>(`/oauth/providers/${provider}`);
    return response.data;
  },

  // Update provider config (admin)
  updateProvider: async (
    provider: string,
    data: { enabled?: boolean; client_id?: string; client_secret?: string }
  ): Promise<OAuthProvider> => {
    const response = await apiClient.put<OAuthProvider>(`/oauth/providers/${provider}`, data);
    return response.data;
  },

  // Get linked accounts for current user
  getLinkedAccounts: async (): Promise<LinkedAccount[]> => {
    const response = await apiClient.get<LinkedAccount[]>('/oauth/accounts');
    return response.data;
  },

  // Unlink an OAuth account
  unlinkAccount: async (provider: string): Promise<void> => {
    await apiClient.delete(`/oauth/accounts/${provider}`);
  },

  // Get OAuth login URL (redirect to provider)
  getLoginUrl: (provider: string): string => {
    return `/api/oauth/${provider}/login`;
  },

  // Get OAuth link URL (for linking existing account)
  getLinkUrl: (provider: string): string => {
    return `/api/oauth/link/${provider}`;
  },
};
