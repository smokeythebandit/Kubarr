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
  custom_config?: Record<string, any>;
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
  status: 'queued' | 'running' | 'succeeded' | 'failed' | string;
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
