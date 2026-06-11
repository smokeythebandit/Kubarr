import apiClient from './client';
import type { AppHealth, PodMetrics, PodStatus, ServiceEndpoint } from '../types';

export interface AppMetrics {
  app_name: string;
  namespace: string;
  cpu_usage_cores: number;
  memory_usage_bytes: number;
  memory_usage_mb: number;
  cpu_usage_percent?: number;
  memory_usage_percent?: number;
  network_receive_bytes_per_sec: number;
  network_transmit_bytes_per_sec: number;
}

export interface ClusterMetrics {
  total_cpu_cores: number;
  total_memory_bytes: number;
  used_cpu_cores: number;
  used_memory_bytes: number;
  cpu_usage_percent: number;
  memory_usage_percent: number;
  container_count: number;
  pod_count: number;
  network_receive_bytes_per_sec: number;
  network_transmit_bytes_per_sec: number;
  // Storage metrics
  total_storage_bytes: number;
  used_storage_bytes: number;
  storage_usage_percent: number;
}

export interface MetricsAvailability {
  available: boolean;
  message: string;
}

export interface TimeSeriesPoint {
  timestamp: number;
  value: number;
}

export interface AppHistoricalMetrics {
  app_name: string;
  namespace: string;
  cpu_series: TimeSeriesPoint[];
  memory_series: TimeSeriesPoint[];
  network_rx_series: TimeSeriesPoint[];
  network_tx_series: TimeSeriesPoint[];
  cpu_usage_cores: number;
  memory_usage_bytes: number;
  memory_usage_mb: number;
  network_receive_bytes_per_sec: number;
  network_transmit_bytes_per_sec: number;
}

export interface PodStatusInfo {
  name: string;
  namespace: string;
  status: string;
  ready: boolean;
  restarts: number;
  age: string;
  node: string;
  ip: string;
  cpu_usage?: number;     // CPU usage in cores (from VictoriaMetrics)
  memory_usage?: number;  // Memory usage in bytes (from VictoriaMetrics)
}

export interface AppDetailMetrics {
  app_name: string;
  namespace: string;
  historical: AppHistoricalMetrics;
  pods: PodStatusInfo[];
}

export interface ClusterNetworkHistory {
  combined_series: TimeSeriesPoint[];
  rx_series: TimeSeriesPoint[];
  tx_series: TimeSeriesPoint[];
}

export interface ClusterMetricsHistory {
  cpu_series: TimeSeriesPoint[];
  memory_series: TimeSeriesPoint[];
  storage_series: TimeSeriesPoint[];
  pod_series: TimeSeriesPoint[];
  container_series: TimeSeriesPoint[];
}

export const monitoringApi = {
  // Get pod status
  getPodStatus: async (namespace: string = 'media', app?: string): Promise<PodStatus[]> => {
    const response = await apiClient.get<PodStatus[]>('/monitoring/pods', {
      params: { namespace, app },
    });
    return response.data;
  },

  // Get pod metrics
  getMetrics: async (namespace: string = 'media', app?: string): Promise<PodMetrics[]> => {
    const response = await apiClient.get<PodMetrics[]>('/monitoring/metrics', {
      params: { namespace, app },
    });
    return response.data;
  },

  // Get app health
  getAppHealth: async (appName: string, namespace: string = 'media'): Promise<AppHealth> => {
    const response = await apiClient.get<AppHealth>(`/monitoring/health/${appName}`, {
      params: { namespace },
    });
    return response.data;
  },

  // Get service endpoints
  getEndpoints: async (
    appName: string,
    namespace: string = 'media'
  ): Promise<ServiceEndpoint[]> => {
    const response = await apiClient.get<ServiceEndpoint[]>(
      `/monitoring/endpoints/${appName}`,
      {
        params: { namespace },
      }
    );
    return response.data;
  },

  // Check if VictoriaMetrics is available
  checkMetricsAvailable: async (): Promise<MetricsAvailability> => {
    const response = await apiClient.get<MetricsAvailability>('/monitoring/vm/available');
    return response.data;
  },

  // Get metrics for all apps from VictoriaMetrics
  getAppMetrics: async (): Promise<AppMetrics[]> => {
    const response = await apiClient.get<AppMetrics[]>('/monitoring/vm/apps');
    return response.data;
  },

  // Get cluster-wide metrics from VictoriaMetrics
  getClusterMetrics: async (): Promise<ClusterMetrics> => {
    const response = await apiClient.get<ClusterMetrics>('/monitoring/vm/cluster');
    return response.data;
  },

  // Get detailed metrics for a specific app
  getAppDetailMetrics: async (appName: string, duration: string = '1h'): Promise<AppDetailMetrics> => {
    const response = await apiClient.get<AppDetailMetrics>(`/monitoring/vm/app/${appName}`, {
      params: { duration },
    });
    return response.data;
  },

  // Get cluster-wide network history for sparkline charts
  getClusterNetworkHistory: async (duration: string = '15m'): Promise<ClusterNetworkHistory> => {
    const response = await apiClient.get<ClusterNetworkHistory>('/monitoring/vm/cluster/network-history', {
      params: { duration },
    });
    return response.data;
  },

  // Get cluster-wide metrics history for sparkline charts (CPU, Memory, Storage, Pods, Containers)
  getClusterMetricsHistory: async (duration: string = '15m'): Promise<ClusterMetricsHistory> => {
    const response = await apiClient.get<ClusterMetricsHistory>('/monitoring/vm/cluster/metrics-history', {
      params: { duration },
    });
    return response.data;
  },
};
