export interface PodStatus {
  name: string;
  app: string;
  namespace: string;
  status: string;
  ready: boolean;
  restart_count: number;
  age: string;
  node: string | null;
  ip: string | null;
}

export interface PodMetrics {
  name: string;
  namespace: string;
  cpu_usage: string;
  memory_usage: string;
  timestamp: string;
}

export interface ServiceEndpoint {
  name: string;
  namespace: string;
  port: number;
  target_port: number;
  port_forward_command: string;
  url: string | null;
  type: string;
}

export interface AppHealth {
  app_name: string;
  namespace: string;
  healthy: boolean;
  pods: PodStatus[];
  metrics: PodMetrics[] | null;
  endpoints: ServiceEndpoint[];
  message: string | null;
}
