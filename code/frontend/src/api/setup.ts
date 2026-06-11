import axios from 'axios';

// Separate client for setup endpoints (no auth required)
const setupClient = axios.create({
  baseURL: import.meta.env.VITE_API_URL || '/api',
  timeout: 30000, // Longer timeout for setup operations
  headers: {
    'Content-Type': 'application/json',
  },
});

export interface SetupStatusResponse {
  setup_required: boolean;
  bootstrap_complete: boolean;
  server_configured: boolean;
  admin_user_exists: boolean;
  storage_configured: boolean;
}

export interface SetupRequest {
  admin_username: string;
  admin_email: string;
  admin_password: string;
}

export interface SetupResult {
  success: boolean;
  message: string;
  data: {
    admin_user: {
      id: number;
      username: string;
      email: string;
    };
    server: {
      name: string;
      storage_path: string;
    };
  };
}

export type StorageMode = 'external_nfs' | 'managed_nfs';

export interface StorageConfigRequest {
  mode: StorageMode;
  external_nfs?: {
    server: string;
    export_path: string;
  } | null;
  managed_nfs?: {
    storage_class?: string | null;
    size: string;
  } | null;
}

export interface StorageValidationCheck {
  name: string;
  passed: boolean;
  warning: boolean;
  message: string;
}

export interface StorageValidationResponse {
  valid: boolean;
  message: string;
  checks: StorageValidationCheck[];
  warnings: string[];
  benchmark?: {
    bytes: number;
    message: string;
  } | null;
}

export interface StorageConfigResponse {
  mode: StorageMode;
  mount_path: string;
  uid: number;
  gid: number;
  fs_group: number;
  config_json: Record<string, unknown>;
  validation_json?: StorageValidationResponse | Record<string, unknown> | null;
  validated: boolean;
  created_at?: string | null;
  updated_at?: string | null;
}

export interface GeneratedCredentials {
  admin_username: string;
  admin_email: string;
  admin_password: string;
}

export interface PathValidationResult {
  valid: boolean;
  exists: boolean;
  writable: boolean;
  message: string;
}

// Bootstrap types
export interface ComponentStatus {
  component: string;
  display_name: string;
  status: 'pending' | 'installing' | 'healthy' | 'failed';
  message?: string;
  error?: string;
}

export interface BootstrapStatusResponse {
  components: ComponentStatus[];
  complete: boolean;
  started: boolean;
}

export interface BootstrapStartResponse {
  message: string;
  started: boolean;
}

// Directory browsing types (setup)
export interface SetupDirectoryEntry {
  name: string;
  path: string;
}

export interface SetupBrowseResponse {
  path: string;
  parent: string | null;
  directories: SetupDirectoryEntry[];
}

// Server config types
export interface ServerConfigRequest {
  name: string;
  storage_path?: string;
}

export interface ServerConfigResponse {
  name: string;
  storage_path: string;
}

export const setupApi = {
  // Check if setup is required (accessible without auth)
  checkRequired: async (): Promise<{ setup_required: boolean; database_pending: boolean }> => {
    const response = await setupClient.get('/setup/required');
    return response.data;
  },

  // Get setup status (only accessible during setup)
  getStatus: async (): Promise<SetupStatusResponse> => {
    const response = await setupClient.get<SetupStatusResponse>('/setup/status');
    return response.data;
  },

  // Initialize the setup (create admin user)
  initialize: async (data: SetupRequest): Promise<SetupResult> => {
    const response = await setupClient.post<SetupResult>('/setup/initialize', data);
    return response.data;
  },

  // Generate random credentials
  generateCredentials: async (): Promise<GeneratedCredentials> => {
    const response = await setupClient.get<GeneratedCredentials>('/setup/generate-credentials');
    return response.data;
  },

  // Validate storage path
  validatePath: async (path: string): Promise<PathValidationResult> => {
    const response = await setupClient.post<PathValidationResult>('/setup/validate-path', null, {
      params: { path },
    });
    return response.data;
  },

  // Browse directories on the host filesystem
  browsePath: async (path: string = '/'): Promise<SetupBrowseResponse> => {
    const response = await setupClient.get<SetupBrowseResponse>('/setup/browse', {
      params: { path },
    });
    return response.data;
  },

  // Bootstrap endpoints
  getBootstrapStatus: async (): Promise<BootstrapStatusResponse> => {
    const response = await setupClient.get<BootstrapStatusResponse>('/setup/bootstrap/status');
    return response.data;
  },

  startBootstrap: async (): Promise<BootstrapStartResponse> => {
    const response = await setupClient.post<BootstrapStartResponse>('/setup/bootstrap/start');
    return response.data;
  },

  retryBootstrapComponent: async (component: string): Promise<BootstrapStartResponse> => {
    const response = await setupClient.post<BootstrapStartResponse>(`/setup/bootstrap/retry/${component}`);
    return response.data;
  },

  // Storage setup endpoints
  getStorageConfig: async (): Promise<StorageConfigResponse | null> => {
    const response = await setupClient.get<StorageConfigResponse | null>('/setup/storage');
    return response.data;
  },

  validateStorage: async (config: StorageConfigRequest): Promise<StorageValidationResponse> => {
    const response = await setupClient.post<StorageValidationResponse>('/setup/storage/validate', config);
    return response.data;
  },

  configureStorage: async (config: StorageConfigRequest): Promise<StorageConfigResponse> => {
    const response = await setupClient.post<StorageConfigResponse>('/setup/storage', config);
    return response.data;
  },

  provisionStorage: async (): Promise<{ success: boolean; message: string; mount_path: string; claim: string }> => {
    const response = await setupClient.post('/setup/storage/provision');
    return response.data;
  },

  // Server config endpoints
  getServerConfig: async (): Promise<ServerConfigResponse | null> => {
    const response = await setupClient.get<ServerConfigResponse | null>('/setup/server');
    return response.data;
  },

  configureServer: async (config: ServerConfigRequest): Promise<ServerConfigResponse> => {
    const response = await setupClient.post<ServerConfigResponse>('/setup/server', config);
    return response.data;
  },
};
