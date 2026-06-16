import apiClient from './client';

export type DynamicDnsCapabilities = {
  a_records: boolean;
  aaaa_records: boolean;
  cname_records: boolean;
  txt_records: boolean;
  wildcard_records: boolean;
};

export type DynamicDnsProfile = {
  id: number;
  name: string;
  provider: string;
  capabilities: DynamicDnsCapabilities;
  config: Record<string, string>;
  enabled: boolean;
  status: string;
  last_error: string | null;
  created_at: string;
  updated_at: string;
};

export type DynamicDnsProfileRequest = {
  name: string;
  provider: string;
  capabilities: DynamicDnsCapabilities;
  config: Record<string, string>;
  enabled: boolean;
};

export type LetsEncryptProfile = {
  id: number;
  name: string;
  email: string;
  environment: 'staging' | 'production' | string;
  challenge_type: 'http01' | 'dns01' | string;
  dns_profile_id: number | null;
  renewal_enabled: boolean;
  enabled: boolean;
  status: string;
  last_error: string | null;
  created_at: string;
  updated_at: string;
};

export type LetsEncryptProfileRequest = {
  name: string;
  email: string;
  environment: string;
  challenge_type: string;
  dns_profile_id: number | null;
  renewal_enabled: boolean;
  enabled: boolean;
};

export type DomainConfig = {
  id: number;
  domain: string;
  kind: 'root' | 'wildcard' | 'exact' | string;
  scope: 'public' | 'private' | 'both' | string;
  primary: boolean;
  enabled: boolean;
  dns_mode: 'manual' | 'dynamic_dns' | string;
  ddns_profile_id: number | null;
  dns_status: string;
  tls_mode: 'none' | 'manual' | 'letsencrypt' | string;
  letsencrypt_profile_id: number | null;
  tls_secret_name: string | null;
  certificate_status: string;
  certificate_expires_at: string | null;
  created_at: string;
  updated_at: string;
};

export type DomainRequest = {
  domain: string;
  kind: string;
  scope: string;
  primary: boolean;
  enabled: boolean;
  dns_mode: string;
  ddns_profile_id: number | null;
  tls_mode: string;
  letsencrypt_profile_id: number | null;
  tls_secret_name: string | null;
};

export type AppDomainAssignment = {
  id: number;
  app_name: string;
  domain_id: number;
  route_mode: 'path' | 'subdomain' | 'exact_host' | string;
  hostname: string | null;
  path_prefix: string | null;
  primary: boolean;
  enabled: boolean;
  created_at: string;
  updated_at: string;
};

export type AppDomainAssignmentRequest = {
  app_name: string;
  domain_id: number;
  route_mode: string;
  hostname: string | null;
  path_prefix: string | null;
  primary: boolean;
  enabled: boolean;
};

export const defaultDnsCapabilities = (): DynamicDnsCapabilities => ({
  a_records: true,
  aaaa_records: true,
  cname_records: false,
  txt_records: false,
  wildcard_records: false,
});

export const domainsApi = {
  listDomains: async () => (await apiClient.get<DomainConfig[]>('/domains')).data,
  createDomain: async (request: DomainRequest) => (await apiClient.post<DomainConfig>('/domains', request)).data,
  updateDomain: async (id: number, request: DomainRequest) => (await apiClient.put<DomainConfig>(`/domains/${id}`, request)).data,
  deleteDomain: async (id: number) => apiClient.delete(`/domains/${id}`),

  listAssignments: async () => (await apiClient.get<AppDomainAssignment[]>('/domains/assignments')).data,
  createAssignment: async (request: AppDomainAssignmentRequest) => (await apiClient.post<AppDomainAssignment>('/domains/assignments', request)).data,
  updateAssignment: async (id: number, request: AppDomainAssignmentRequest) => (await apiClient.put<AppDomainAssignment>(`/domains/assignments/${id}`, request)).data,
  deleteAssignment: async (id: number) => apiClient.delete(`/domains/assignments/${id}`),

  listDdnsProfiles: async () => (await apiClient.get<DynamicDnsProfile[]>('/domains/ddns-profiles')).data,
  createDdnsProfile: async (request: DynamicDnsProfileRequest) => (await apiClient.post<DynamicDnsProfile>('/domains/ddns-profiles', request)).data,
  updateDdnsProfile: async (id: number, request: DynamicDnsProfileRequest) => (await apiClient.put<DynamicDnsProfile>(`/domains/ddns-profiles/${id}`, request)).data,
  deleteDdnsProfile: async (id: number) => apiClient.delete(`/domains/ddns-profiles/${id}`),

  listLetsEncryptProfiles: async () => (await apiClient.get<LetsEncryptProfile[]>('/domains/letsencrypt-profiles')).data,
  createLetsEncryptProfile: async (request: LetsEncryptProfileRequest) => (await apiClient.post<LetsEncryptProfile>('/domains/letsencrypt-profiles', request)).data,
  updateLetsEncryptProfile: async (id: number, request: LetsEncryptProfileRequest) => (await apiClient.put<LetsEncryptProfile>(`/domains/letsencrypt-profiles/${id}`, request)).data,
  deleteLetsEncryptProfile: async (id: number) => apiClient.delete(`/domains/letsencrypt-profiles/${id}`),
};
