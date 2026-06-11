import apiClient from './client';

export interface AuditLog {
  id: number;
  timestamp: string;
  user_id: number | null;
  username: string | null;
  action: string;
  resource_type: string;
  resource_id: string | null;
  details: string | null;
  ip_address: string | null;
  user_agent: string | null;
  success: boolean;
  error_message: string | null;
}

export interface AuditLogResponse {
  logs: AuditLog[];
  total: number;
  page: number;
  per_page: number;
  total_pages: number;
}

export interface AuditLogQuery {
  page?: number;
  per_page?: number;
  user_id?: number;
  action?: string;
  resource_type?: string;
  success?: boolean;
  from?: string;
  to?: string;
  search?: string;
}

export interface ActionCount {
  action: string;
  count: number;
}

export interface AuditStats {
  total_events: number;
  successful_events: number;
  failed_events: number;
  events_today: number;
  events_this_week: number;
  top_actions: ActionCount[];
  recent_failures: AuditLog[];
}

export interface ClearLogsResponse {
  deleted: number;
  message: string;
}

export const auditApi = {
  getLogs: async (query: AuditLogQuery = {}): Promise<AuditLogResponse> => {
    const params = new URLSearchParams();
    Object.entries(query).forEach(([key, value]) => {
      if (value !== undefined && value !== null && value !== '') {
        params.append(key, String(value));
      }
    });
    const response = await apiClient.get(`/audit?${params.toString()}`);
    return response.data;
  },

  getStats: async (): Promise<AuditStats> => {
    const response = await apiClient.get('/audit/stats');
    return response.data;
  },

  clearOldLogs: async (days: number = 90): Promise<ClearLogsResponse> => {
    const response = await apiClient.post('/audit/clear', { days });
    return response.data;
  },
};
