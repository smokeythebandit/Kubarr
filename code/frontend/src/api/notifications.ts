import apiClient from './client';

// ============================================================================
// Types
// ============================================================================

export interface Notification {
  id: number;
  title: string;
  message: string;
  event_type: string | null;
  severity: 'info' | 'warning' | 'critical';
  read: boolean;
  created_at: string;
}

export interface InboxResponse {
  notifications: Notification[];
  total: number;
  unread: number;
}

export interface UnreadCountResponse {
  count: number;
}

export interface NotificationChannel {
  channel_type: string;
  enabled: boolean;
  config: Record<string, unknown>;
  created_at: string;
  updated_at: string;
}

export interface NotificationEvent {
  event_type: string;
  enabled: boolean;
  severity: string;
}

export interface UserNotificationPref {
  channel_type: string;
  enabled: boolean;
  destination: string | null;
  verified: boolean;
}

export interface NotificationLog {
  id: number;
  user_id: number | null;
  channel_type: string;
  event_type: string;
  recipient: string | null;
  status: string;
  error_message: string | null;
  created_at: string;
}

export interface LogsResponse {
  logs: NotificationLog[];
  total: number;
}

export interface TestChannelResponse {
  success: boolean;
  error: string | null;
}

// ============================================================================
// User Inbox API
// ============================================================================

export const notificationsApi = {
  // Get user's notification inbox
  getInbox: async (limit = 20, offset = 0): Promise<InboxResponse> => {
    const response = await apiClient.get(`/notifications/inbox?limit=${limit}&offset=${offset}`);
    return response.data;
  },

  // Get unread notification count
  getUnreadCount: async (): Promise<number> => {
    const response = await apiClient.get('/notifications/inbox/count');
    return response.data.count;
  },

  // Mark a notification as read
  markAsRead: async (id: number): Promise<void> => {
    await apiClient.post(`/notifications/inbox/${id}/read`);
  },

  // Mark all notifications as read
  markAllAsRead: async (): Promise<void> => {
    await apiClient.post('/notifications/inbox/read-all');
  },

  // Delete a notification
  deleteNotification: async (id: number): Promise<void> => {
    await apiClient.delete(`/notifications/inbox/${id}`);
  },

  // ============================================================================
  // Admin: Channel Configuration
  // ============================================================================

  // Get all notification channels (admin)
  getChannels: async (): Promise<NotificationChannel[]> => {
    const response = await apiClient.get('/notifications/channels');
    return response.data;
  },

  // Get a specific channel configuration (admin)
  getChannel: async (channelType: string): Promise<NotificationChannel> => {
    const response = await apiClient.get(`/notifications/channels/${channelType}`);
    return response.data;
  },

  // Update channel configuration (admin)
  updateChannel: async (
    channelType: string,
    data: { enabled?: boolean; config?: Record<string, unknown> }
  ): Promise<NotificationChannel> => {
    const response = await apiClient.put(`/notifications/channels/${channelType}`, data);
    return response.data;
  },

  // Test a notification channel (admin)
  testChannel: async (channelType: string, destination: string): Promise<TestChannelResponse> => {
    const response = await apiClient.post(`/notifications/channels/${channelType}/test`, {
      destination,
    });
    return response.data;
  },

  // ============================================================================
  // Admin: Event Settings
  // ============================================================================

  // Get all event notification settings (admin)
  getEvents: async (): Promise<NotificationEvent[]> => {
    const response = await apiClient.get('/notifications/events');
    return response.data;
  },

  // Update event notification settings (admin)
  updateEvent: async (
    eventType: string,
    data: { enabled?: boolean; severity?: string }
  ): Promise<NotificationEvent> => {
    const response = await apiClient.put(`/notifications/events/${eventType}`, data);
    return response.data;
  },

  // ============================================================================
  // User Preferences
  // ============================================================================

  // Get user's notification preferences
  getPreferences: async (): Promise<UserNotificationPref[]> => {
    const response = await apiClient.get('/notifications/preferences');
    return response.data;
  },

  // Update user's notification preference for a channel
  updatePreference: async (
    channelType: string,
    data: { enabled?: boolean; destination?: string }
  ): Promise<UserNotificationPref> => {
    const response = await apiClient.put(`/notifications/preferences/${channelType}`, data);
    return response.data;
  },

  // ============================================================================
  // Admin: Logs
  // ============================================================================

  // Get notification delivery logs (admin)
  getLogs: async (params?: {
    limit?: number;
    offset?: number;
    channel_type?: string;
    status?: string;
  }): Promise<LogsResponse> => {
    const searchParams = new URLSearchParams();
    if (params) {
      Object.entries(params).forEach(([key, value]) => {
        if (value !== undefined && value !== null) {
          searchParams.append(key, String(value));
        }
      });
    }
    const response = await apiClient.get(`/notifications/logs?${searchParams.toString()}`);
    return response.data;
  },
};
