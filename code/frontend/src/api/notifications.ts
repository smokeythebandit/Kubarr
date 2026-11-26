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
  /** Unread notifications with severity `critical`. Older servers omit it. */
  critical?: number;
  /** Unread warnings + criticals ("needs you"). Older servers omit it. */
  attention?: number;
}

export interface UnreadCounts {
  count: number;
  critical: number;
  attention: number;
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

/** External channels an event can be routed to. The bell is not routed. */
export type RouteChannel = 'telegram' | 'email' | 'messagebird';

export const ROUTE_CHANNELS: RouteChannel[] = ['telegram', 'email', 'messagebird'];

/** The signed-in user's quiet hours. Minutes are after local midnight. */
export interface QuietHours {
  enabled: boolean;
  from_minute: number;
  to_minute: number;
  /** IANA time zone, e.g. Europe/Amsterdam. */
  timezone: string;
  /** Critical notifications are sent straight away during quiet hours. */
  critical_bypass: boolean;
  /** False until the user saves; the values are then server defaults. */
  configured: boolean;
}

/** Which external channels an event is delivered to. */
export interface NotificationRoute {
  event_type: string;
  channels: Record<RouteChannel, boolean>;
}

export interface UserNotificationPref {
  channel_type: string;
  enabled: boolean;
  destination: string | null;
  verified: boolean;
  /** A verification code was sent and can still be confirmed. Older servers omit it. */
  verification_pending?: boolean;
}

export interface VerifyStartResponse {
  /** Masked destination the code was sent to. */
  destination: string;
  expires_in_seconds: number;
  /** Seconds before another code may be requested. */
  resend_after_seconds: number;
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

  // Unread count plus how many of those are critical (drives the red badge)
  getUnreadCounts: async (): Promise<UnreadCounts> => {
    const response = await apiClient.get<UnreadCountResponse>('/notifications/inbox/count');
    return {
      count: response.data.count ?? 0,
      critical: response.data.critical ?? 0,
      attention: response.data.attention ?? 0,
    };
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
  // Admin: Routing (which channels each event goes to)
  // ============================================================================

  // Get every event's external channel routes (admin)
  getRoutes: async (): Promise<NotificationRoute[]> => {
    const response = await apiClient.get('/notifications/routes');
    return response.data;
  },

  // Change some of an event's routes; channels left out keep their route (admin)
  updateRoute: async (
    eventType: string,
    channels: Partial<Record<RouteChannel, boolean>>
  ): Promise<NotificationRoute> => {
    const response = await apiClient.put(`/notifications/routes/${eventType}`, { channels });
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

  // Send a 6-digit code to the user's destination for a channel.
  // Rejects with 429 (Retry-After header) when rate limited.
  startVerification: async (channelType: string): Promise<VerifyStartResponse> => {
    const response = await apiClient.post(`/notifications/preferences/${channelType}/verify/start`);
    return response.data;
  },

  // Confirm the code; resolves with the now-verified preference, 400 otherwise.
  confirmVerification: async (channelType: string, code: string): Promise<UserNotificationPref> => {
    const response = await apiClient.post(`/notifications/preferences/${channelType}/verify/confirm`, { code });
    return response.data;
  },

  // ============================================================================
  // Quiet hours (own user)
  // ============================================================================

  getQuietHours: async (): Promise<QuietHours> => {
    const response = await apiClient.get('/notifications/quiet-hours');
    return response.data;
  },

  // Rejects with 400 for equal start/end times or an unknown time zone.
  updateQuietHours: async (data: Omit<QuietHours, 'configured'>): Promise<QuietHours> => {
    const response = await apiClient.put('/notifications/quiet-hours', data);
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
