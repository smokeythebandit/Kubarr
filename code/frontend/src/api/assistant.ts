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

// ---------------------------------------------------------------------------
// Chat
// ---------------------------------------------------------------------------

/** What a conversation or question is about. */
export interface ChatContext {
  page?: string;
  app?: string;
  /** Why the conversation was opened (set when it is created). */
  kind?: 'app' | 'operation';
  operation_id?: string;
}

/** A tool the assistant used for an answer (never its result). */
export interface AssistantSource {
  tool: string;
  target?: string;
  label: string;
}

export type ChatMessageStatus = 'complete' | 'streaming' | 'interrupted' | 'error';

export interface ChatMessage {
  id: number;
  role: 'user' | 'assistant';
  text: string;
  sources: AssistantSource[];
  status: ChatMessageStatus;
  error?: string;
  created_at: string;
}

export interface ConversationSummary {
  id: string;
  title: string;
  context: ChatContext;
  created_at: string;
  updated_at: string;
}

export type ActionStatus = 'pending' | 'expired' | 'running' | 'done' | 'failed' | 'retried' | 'cancelled';

/** What a proposed action will do, computed by Kubarr (never by the model). */
export interface ActionSummary {
  title: string;
  effect: string;
  from_version?: string;
  to_version?: string;
}

/** An action the assistant proposed; it only runs when the person clicks Run. */
export interface ActionCard {
  id: string;
  kind: string;
  app: string;
  /** The answer that proposed it. */
  message_id?: number;
  summary: ActionSummary;
  status: ActionStatus;
  expires_at: string;
  operation_id?: string;
  error?: string;
  /** Why the assistant proposed it. */
  reason?: string;
  created_at?: string;
}

export interface RunActionResult {
  status: 'running' | 'done';
  operation_id?: string;
}

export interface ConversationDetail extends ConversationSummary {
  messages: ChatMessage[];
  actions?: ActionCard[];
}

/** One server-sent event of an answer. */
export type ChatEvent =
  | { event: 'status'; text: string }
  | { event: 'text_delta'; text: string }
  | { event: 'sources'; items: AssistantSource[] }
  | { event: 'action_card'; card: ActionCard }
  | { event: 'done'; message_id: number }
  | { event: 'error'; message: string };

/** The question was refused before streaming (budget, rate limit, setup). */
export class AssistantRequestError extends Error {
  constructor(public status: number, message: string) {
    super(message);
    this.name = 'AssistantRequestError';
  }
}

function toEvent(name: string, data: string): ChatEvent | null {
  let payload: Record<string, unknown>;
  try {
    payload = JSON.parse(data) as Record<string, unknown>;
  } catch {
    return null;
  }
  switch (name) {
    case 'status':
    case 'text_delta':
      return typeof payload.text === 'string' ? { event: name, text: payload.text } : null;
    case 'sources':
      return Array.isArray(payload.items) ? { event: 'sources', items: payload.items as AssistantSource[] } : null;
    case 'action_card':
      return typeof payload.id === 'string' && typeof payload.kind === 'string'
        ? { event: 'action_card', card: payload as unknown as ActionCard }
        : null;
    case 'done':
      return typeof payload.message_id === 'number' ? { event: 'done', message_id: payload.message_id } : null;
    case 'error':
      return typeof payload.message === 'string' ? { event: 'error', message: payload.message } : null;
    default:
      return null;
  }
}

/** Incremental SSE parser: push decoded text, get complete events. */
export class SseReader {
  private buffer = '';

  push(text: string): ChatEvent[] {
    this.buffer += text.replace(/\r\n/g, '\n');
    const events: ChatEvent[] = [];
    let end: number;
    while ((end = this.buffer.indexOf('\n\n')) !== -1) {
      const block = this.buffer.slice(0, end);
      this.buffer = this.buffer.slice(end + 2);
      let name = 'message';
      const data: string[] = [];
      for (const line of block.split('\n')) {
        if (line.startsWith('event:')) name = line.slice(6).trim();
        else if (line.startsWith('data:')) data.push(line.slice(5).replace(/^ /, ''));
      }
      const event = toEvent(name, data.join('\n'));
      if (event) events.push(event);
    }
    return events;
  }
}

const apiBase = (): string => import.meta.env.VITE_API_URL || '/api';

export const assistantChatApi = {
  listConversations: async (): Promise<ConversationSummary[]> => {
    const response = await apiClient.get<ConversationSummary[]>('/assistant/conversations');
    return response.data;
  },

  createConversation: async (context?: ChatContext): Promise<ConversationSummary> => {
    const response = await apiClient.post<ConversationSummary>('/assistant/conversations', { context });
    return response.data;
  },

  getConversation: async (id: string): Promise<ConversationDetail> => {
    const response = await apiClient.get<ConversationDetail>(`/assistant/conversations/${encodeURIComponent(id)}`);
    return response.data;
  },

  deleteConversation: async (id: string): Promise<void> => {
    await apiClient.delete(`/assistant/conversations/${encodeURIComponent(id)}`);
  },

  /** Run a proposed action (the only way it is carried out). */
  runAction: async (id: string): Promise<RunActionResult> => {
    const response = await apiClient.post<RunActionResult>(`/assistant/actions/${encodeURIComponent(id)}/run`);
    return response.data;
  },

  cancelAction: async (id: string): Promise<void> => {
    await apiClient.post(`/assistant/actions/${encodeURIComponent(id)}/cancel`);
  },

  /**
   * Ask a question and call `onEvent` for each event as the answer streams.
   * Resolves when the stream ends (normally or because it broke); rejects
   * with {@link AssistantRequestError} when the question was refused.
   */
  ask: async (
    conversationId: string,
    text: string,
    context: ChatContext | undefined,
    onEvent: (event: ChatEvent) => void,
    signal?: AbortSignal,
  ): Promise<void> => {
    const response = await fetch(`${apiBase()}/assistant/conversations/${encodeURIComponent(conversationId)}/messages`, {
      method: 'POST',
      credentials: 'include',
      headers: { 'Content-Type': 'application/json', Accept: 'text/event-stream' },
      body: JSON.stringify({ text, context }),
      signal,
    });
    if (!response.ok || !response.body) {
      let detail = 'The assistant could not answer right now.';
      try {
        const body = (await response.json()) as { detail?: unknown };
        if (typeof body.detail === 'string') detail = body.detail;
      } catch {
        // keep the generic sentence
      }
      throw new AssistantRequestError(response.status, detail);
    }
    const reader = response.body.getReader();
    const decoder = new TextDecoder();
    const sse = new SseReader();
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      for (const event of sse.push(decoder.decode(value, { stream: true }))) onEvent(event);
    }
    for (const event of sse.push(decoder.decode())) onEvent(event);
  },
};
