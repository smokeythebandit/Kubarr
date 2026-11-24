import apiClient from './client';

export interface UpdateSettings {
  enabled: boolean;
  /** Local wall-clock start of the window, "HH:MM". */
  window_start: string;
  window_minutes: number;
  /** IANA timezone name the window is evaluated in. */
  timezone: string;
  health_timeout_minutes: number;
}

export interface UpdateSettingsResponse extends UpdateSettings {
  /** Start (UTC ISO) of the running or next window, or null. */
  next_window_start: string | null;
}

export const updatesApi = {
  getSettings: async (): Promise<UpdateSettingsResponse> => {
    const response = await apiClient.get<UpdateSettingsResponse>('/updates/settings');
    return response.data;
  },

  saveSettings: async (settings: UpdateSettings): Promise<UpdateSettingsResponse> => {
    const response = await apiClient.put<UpdateSettingsResponse>('/updates/settings', settings);
    return response.data;
  },
};
