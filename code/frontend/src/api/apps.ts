import apiClient from './client';
import type { AppConfig, DeploymentRequest, DeploymentStatus } from '../types';

// Export type for convenience
export type App = AppConfig;

// Named exports for direct usage
export const getCatalog = async (): Promise<AppConfig[]> => {
  const response = await apiClient.get<AppConfig[]>('/apps/catalog');
  return response.data;
};

export const appsApi = {
  // Get all apps in catalog
  getCatalog: async (): Promise<AppConfig[]> => {
    const response = await apiClient.get<AppConfig[]>('/apps/catalog');
    return response.data;
  },

  // Get specific app from catalog
  getApp: async (appName: string): Promise<AppConfig> => {
    const response = await apiClient.get<AppConfig>(`/apps/catalog/${appName}`);
    return response.data;
  },

  // Get installed apps
  getInstalled: async (): Promise<string[]> => {
    const response = await apiClient.get<string[]>('/apps/installed');
    return response.data;
  },

  // Install app
  install: async (request: DeploymentRequest): Promise<DeploymentStatus> => {
    const response = await apiClient.post<DeploymentStatus>('/apps/install', request);
    return response.data;
  },

  // Delete app
  delete: async (appName: string): Promise<{success: boolean, message: string, status: string}> => {
    const response = await apiClient.delete(`/apps/${appName}`);
    return response.data;
  },

  // Check app health
  checkHealth: async (appName: string): Promise<{status: string, healthy: boolean, message: string, deployments?: any[]}> => {
    const response = await apiClient.get(`/apps/${appName}/health`);
    return response.data;
  },

  // Check if app exists
  checkExists: async (appName: string): Promise<{exists: boolean}> => {
    const response = await apiClient.get(`/apps/${appName}/exists`);
    return response.data;
  },

  // Get app status
  getStatus: async (appName: string): Promise<{state: string, message: string}> => {
    const response = await apiClient.get(`/apps/${appName}/status`);
    return response.data;
  },

  // Restart app
  restart: async (appName: string, namespace: string = 'media'): Promise<void> => {
    await apiClient.post(`/apps/${appName}/restart`, null, {
      params: { namespace },
    });
  },

  // Get categories
  getCategories: async (): Promise<string[]> => {
    const response = await apiClient.get<string[]>('/apps/categories');
    return response.data;
  },

  // Get apps by category
  getByCategory: async (category: string): Promise<AppConfig[]> => {
    const response = await apiClient.get<AppConfig[]>(`/apps/category/${category}`);
    return response.data;
  },

  // Log app access - called when user opens an app
  logAccess: async (appName: string): Promise<void> => {
    await apiClient.post(`/apps/${appName}/access`);
  },
};
