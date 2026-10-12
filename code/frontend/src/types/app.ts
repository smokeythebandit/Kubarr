export interface AppConfig {
  name: string;
  display_name: string;
  description: string;
  icon: string | null;
  version: string;
  container_image: string;
  default_port: number;
  resource_requirements: ResourceRequirements;
  environment_variables: Record<string, string>;
  volumes: VolumeConfig[];
  category: string;
  is_system: boolean;
  is_hidden: boolean;
  is_browseable: boolean;
}

export interface ResourceRequirements {
  cpu_request: string;
  cpu_limit: string;
  memory_request: string;
  memory_limit: string;
}

export interface VolumeConfig {
  name: string;
  mount_path: string;
  size: string;
  storage_class: string | null;
}

export interface DeploymentRequest {
  app_name: string;
  namespace?: string;
  chart_version?: string;
  storage?: InstallStorage;
  vpn?: { vpn_provider_id: number; kill_switch_override?: boolean; port_forwarding?: boolean } | null;
  custom_config?: Record<string, string>;
  gpu?: GpuSelection;
}

export interface StorageMount { name: string; source: string; target: string; mode: 'read' | 'read-write' }
export interface PrivateConfig { create: boolean; existing_claim: string; storage_class_name: string; size: string }
export interface InstallStorage { mounts?: StorageMount[]; workloads?: Record<string, { mounts: StorageMount[] }>; config?: PrivateConfig }
export interface InstallOptions {
  app_name: string;
  chart_version: string;
  mounts: StorageMount[] | null;
  workloads: Record<string, StorageMount[]>;
  config: PrivateConfig | null;
  private_claim: string | null;
  required: Record<string, string[]>;
  reserved: Record<string, string[]>;
  vpn_supported: boolean;
  gpu_supported: boolean;
  saved_storage?: InstallStorage;
  saved_chart_version?: string;
}

export interface GpuSelection {
  vendor: 'intel' | 'nvidia' | 'amd';
  node_name: string;
  resource_name: string;
}

export interface DeploymentStatus {
  app_name: string;
  namespace: string;
  status: string;
  message: string | null;
  timestamp: string;
}

export interface AppOperation {
  id: string;
  app_name: string;
  operation: 'install' | 'update' | 'delete' | 'restart' | string;
  status: 'queued' | 'paused' | 'running' | 'succeeded' | 'failed' | 'cancelled' | 'retried' | string;
  stop_requested: boolean;
  message: string | null;
  error: string | null;
  attempts: number;
  created_by: number | null;
  created_at: string;
  started_at: string | null;
  finished_at: string | null;
  updated_at: string;
}

export interface AppState {
  app_name: string;
  namespace: string;
  desired_state: 'installed' | 'removed' | string;
  observed_state: 'not_installed' | 'installing' | 'installed' | 'unhealthy' | 'deleting' | 'failed' | string;
  healthy: boolean;
  message: string | null;
  installed_chart_version: string | null;
  available_chart_version: string | null;
  update_available: boolean;
  last_operation_id: string | null;
  last_checked_at: string | null;
  updated_at: string;
}
