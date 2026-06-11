import apiClient from './client'

export interface LokiLogEntry {
  timestamp: string
  line: string
  level?: string
}

export interface LokiStream {
  labels: Record<string, string>
  entries: LokiLogEntry[]
}

export interface LokiQueryResponse {
  streams: LokiStream[]
  total_entries: number
}

export interface LokiQueryParams {
  query: string
  start?: string
  end?: string
  limit?: number
  direction?: 'forward' | 'backward'
}

export const logsApi = {
  /**
   * Get all available labels from VictoriaLogs
   */
  getLabels: async (): Promise<string[]> => {
    const response = await apiClient.get<string[]>('/logs/loki/labels')
    return response.data
  },

  /**
   * Get all values for a specific label
   */
  getLabelValues: async (label: string): Promise<string[]> => {
    const response = await apiClient.get<string[]>(`/logs/loki/label/${label}/values`)
    return response.data
  },

  /**
   * Get all apps (namespaces) that have logs
   */
  getNamespaces: async (): Promise<string[]> => {
    const response = await apiClient.get<string[]>('/logs/loki/namespaces')
    return response.data
  },

  /**
   * Query logs from VictoriaLogs (uses legacy Loki API endpoint for compatibility)
   */
  queryLogs: async (params: LokiQueryParams): Promise<LokiQueryResponse> => {
    const response = await apiClient.get<LokiQueryResponse>('/logs/loki/query', {
      params: {
        query: params.query,
        start: params.start,
        end: params.end,
        limit: params.limit || 1000,
        direction: params.direction || 'backward',
      },
    })
    return response.data
  },
}
