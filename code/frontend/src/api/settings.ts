import apiClient from './client';

export interface Setting {
  key: string;
  value: string;
  description?: string;
}

export interface SettingsResponse {
  settings: Record<string, Setting>;
}

export const getSettings = async (): Promise<Record<string, Setting>> => {
  const response = await apiClient.get<SettingsResponse>('/settings');
  return response.data.settings;
};

export const getSetting = async (key: string): Promise<Setting> => {
  const response = await apiClient.get<Setting>(`/settings/${key}`);
  return response.data;
};

export const updateSetting = async (key: string, value: string): Promise<Setting> => {
  const response = await apiClient.put<Setting>(`/settings/${key}`, { value });
  return response.data;
};
