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
