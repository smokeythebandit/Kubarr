import apiClient from './client';

// ============================================================================
// Types
// ============================================================================

export type TunnelStatus = 'not_deployed' | 'deploying' | 'running' | 'failed' | 'removing';

export interface CloudflareTunnelConfig {
  id: number;
  name: string;
  /** Always "****" in GET responses */
  tunnel_token: string;
  status: TunnelStatus;
  error: string | null;
  // Fields populated by the guided wizard
  tunnel_id?: string;
  zone_id?: string;
  zone_name?: string;
  subdomain?: string;
  hostname?: string;
  created_at: string;
  updated_at: string;
}

export interface CloudflareTunnelStatus {
  status: TunnelStatus;
  ready_pods: number;
  total_pods: number;
  message: string | null;
}

// ── Wizard types ─────────────────────────────────────────────────────────────

export interface ValidateTokenRequest {
  api_token: string;
}

export interface ZoneInfo {
  id: string;
  name: string;
}

export interface ValidateTokenResponse {
  account_id: string;
  zones: ZoneInfo[];
}

export interface ProvisionRequest {
  name: string;
  api_token: string;
  account_id: string;
  zone_id: string;
  zone_name: string;
  subdomain: string;
}

// ============================================================================
// API client
// ============================================================================

export const cloudflareApi = {
  async getConfig(): Promise<CloudflareTunnelConfig | null> {
    const response = await apiClient.get<CloudflareTunnelConfig | null>('/cloudflare/config');
    return response.data;
  },

  async saveConfig(req: ProvisionRequest): Promise<CloudflareTunnelConfig> {
    const response = await apiClient.put<CloudflareTunnelConfig>('/cloudflare/config', req);
    return response.data;
  },

  async deleteConfig(): Promise<void> {
    await apiClient.delete('/cloudflare/config');
  },

  async getStatus(): Promise<CloudflareTunnelStatus> {
    const response = await apiClient.get<CloudflareTunnelStatus>('/cloudflare/status');
    return response.data;
  },

  async validateToken(req: ValidateTokenRequest): Promise<ValidateTokenResponse> {
    const response = await apiClient.post<ValidateTokenResponse>(
      '/cloudflare/validate-token',
      req,
    );
    return response.data;
  },
};
