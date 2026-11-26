import apiClient from './client';

export type AssistantModel = 'claude-opus-5-5' | 'claude-sonnet-5-5' | 'claude-haiku-5-5';
export type AssistantEffort = 'low' | 'medium' | 'high';

export interface AssistantSettings {
  enabled: boolean;
  model: AssistantModel;
  effort: AssistantEffort;
  monthly_budget_usd: number;
  retention_days: number;
}

export interface AssistantStatus {
  /** An API key is stored (the key itself is never returned). */
  configured: boolean;
  /** The key Secret could not be read (permissions, cluster unreachable). */
  key_error: boolean;
  enabled: boolean;
  model: AssistantModel;
  effort: AssistantEffort;
  /** Spend in the current UTC calendar month. */
  month_spend_usd: number;
  monthly_budget_usd: number;
  /** The budget is used up; the assistant is paused until next month. */
  paused: boolean;
}

export interface KeyStatus {
  configured: boolean;
}

export interface KeyTestResult {
  ok: boolean;
  error?: string;
}

export const ASSISTANT_MODELS: { id: AssistantModel; label: string; hint: string }[] = [
  { id: 'claude-opus-5-5', label: 'Claude Opus 5.5', hint: 'Most capable' },
  { id: 'claude-sonnet-5-5', label: 'Claude Sonnet 5.5', hint: 'Cheaper' },
  { id: 'claude-haiku-5-5', label: 'Claude Haiku 5.5', hint: 'Cheapest' },
];

export const assistantApi = {
  getStatus: async (): Promise<AssistantStatus> => {
    const response = await apiClient.get<AssistantStatus>('/assistant/status');
    return response.data;
  },

  getSettings: async (): Promise<AssistantSettings> => {
    const response = await apiClient.get<AssistantSettings>('/assistant/settings');
    return response.data;
  },

  saveSettings: async (settings: AssistantSettings): Promise<AssistantSettings> => {
    const response = await apiClient.put<AssistantSettings>('/assistant/settings', settings);
    return response.data;
  },

  setKey: async (apiKey: string): Promise<KeyStatus> => {
    const response = await apiClient.put<KeyStatus>('/assistant/key', { api_key: apiKey });
    return response.data;
  },

  removeKey: async (): Promise<KeyStatus> => {
    const response = await apiClient.delete<KeyStatus>('/assistant/key');
    return response.data;
  },

  testKey: async (): Promise<KeyTestResult> => {
    const response = await apiClient.post<KeyTestResult>('/assistant/key/test');
    return response.data;
  },
};
