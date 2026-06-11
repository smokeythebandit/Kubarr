import apiClient from './client';

// ============================================================================
// Types
// ============================================================================

export type VpnType = 'wireguard' | 'openvpn';

export interface VpnProvider {
  id: number;
  name: string;
  vpn_type: VpnType;
  service_provider: string | null;
  enabled: boolean;
  kill_switch: boolean;
  firewall_outbound_subnets: string;
  created_at: string;
  updated_at: string;
  app_count: number;
}

export interface CreateVpnProviderRequest {
  name: string;
  vpn_type: VpnType;
  service_provider?: string;
  credentials: WireGuardCredentials | OpenVpnCredentials;
  enabled?: boolean;
  kill_switch?: boolean;
  firewall_outbound_subnets?: string;
}

export interface UpdateVpnProviderRequest {
  name?: string;
  service_provider?: string;
  credentials?: WireGuardCredentials | OpenVpnCredentials;
  enabled?: boolean;
  kill_switch?: boolean;
  firewall_outbound_subnets?: string;
}

export interface WireGuardCredentials {
  private_key: string;
  addresses?: string[];
  public_key?: string;
  endpoint_ip?: string;
  endpoint_port?: number;
  preshared_key?: string;
}

export interface OpenVpnCredentials {
  username: string;
  password: string;
  server_countries?: string;
  server_cities?: string;
  server_hostnames?: string;
}

export interface AppVpnConfig {
  app_name: string;
  vpn_provider_id: number;
  vpn_provider_name: string;
  kill_switch_override: boolean | null;
  effective_kill_switch: boolean;
  port_forwarding: boolean;
  created_at: string;
  updated_at: string;
}

export interface AssignVpnRequest {
  vpn_provider_id: number;
  kill_switch_override?: boolean;
  port_forwarding?: boolean;
}

export interface SupportedProvider {
  id: string;
  name: string;
  vpn_types: VpnType[];
  description: string;
  supports_port_forwarding: boolean;
}

export interface TestResult {
  success: boolean;
  message: string;
}

// ============================================================================
// VPN Provider API
// ============================================================================

export const vpnApi = {
  // List all VPN providers
  listProviders: async (): Promise<VpnProvider[]> => {
    const response = await apiClient.get<{ providers: VpnProvider[] }>('/vpn/providers');
    return response.data.providers;
  },

  // Get a VPN provider by ID
  getProvider: async (id: number): Promise<VpnProvider> => {
    const response = await apiClient.get<VpnProvider>(`/vpn/providers/${id}`);
    return response.data;
  },

  // Create a new VPN provider
  createProvider: async (data: CreateVpnProviderRequest): Promise<VpnProvider> => {
    const response = await apiClient.post<VpnProvider>('/vpn/providers', data);
    return response.data;
  },

  // Update a VPN provider
  updateProvider: async (id: number, data: UpdateVpnProviderRequest): Promise<VpnProvider> => {
    const response = await apiClient.put<VpnProvider>(`/vpn/providers/${id}`, data);
    return response.data;
  },

  // Delete a VPN provider
  deleteProvider: async (id: number): Promise<void> => {
    await apiClient.delete(`/vpn/providers/${id}`);
  },

  // Test a VPN provider connection
  testProvider: async (id: number): Promise<TestResult> => {
    const response = await apiClient.post<TestResult>(`/vpn/providers/${id}/test`);
    return response.data;
  },

  // List supported VPN service providers
  listSupportedProviders: async (): Promise<SupportedProvider[]> => {
    const response = await apiClient.get<{ providers: SupportedProvider[] }>('/vpn/supported-providers');
    return response.data.providers;
  },
};

// ============================================================================
// App VPN Config API
// ============================================================================

export const appVpnApi = {
  // List all app VPN configurations
  listConfigs: async (): Promise<AppVpnConfig[]> => {
    const response = await apiClient.get<{ configs: AppVpnConfig[] }>('/vpn/apps');
    return response.data.configs;
  },

  // Get app VPN configuration
  getConfig: async (appName: string): Promise<AppVpnConfig | null> => {
    const response = await apiClient.get<AppVpnConfig | null>(`/vpn/apps/${appName}`);
    return response.data;
  },

  // Assign VPN to an app
  assignVpn: async (appName: string, data: AssignVpnRequest): Promise<AppVpnConfig> => {
    const response = await apiClient.put<AppVpnConfig>(`/vpn/apps/${appName}`, data);
    return response.data;
  },

  // Remove VPN from an app
  removeVpn: async (appName: string): Promise<void> => {
    await apiClient.delete(`/vpn/apps/${appName}`);
  },

  // Get the VPN forwarded port for an app
  getForwardedPort: async (appName: string): Promise<{ port: number }> => {
    const response = await apiClient.get<{ port: number }>(`/vpn/apps/${appName}/forwarded-port`);
    return response.data;
  },
};

// ============================================================================
// Helper Functions
// ============================================================================

export function getVpnTypeLabel(type: VpnType): string {
  return type === 'wireguard' ? 'WireGuard' : 'OpenVPN';
}

export function getProviderLabel(providerId: string): string {
  const labels: Record<string, string> = {
    custom: 'Custom',
    airvpn: 'AirVPN',
    expressvpn: 'ExpressVPN',
    ipvanish: 'IPVanish',
    mullvad: 'Mullvad',
    nordvpn: 'NordVPN',
    private_internet_access: 'PIA',
    protonvpn: 'ProtonVPN',
    surfshark: 'Surfshark',
    windscribe: 'Windscribe',
  };
  return labels[providerId] || providerId;
}
