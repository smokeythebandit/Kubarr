import { useQuery } from '@tanstack/react-query';
import { create } from 'zustand';
import { assistantApi, type ChatContext } from '../../api/assistant';
import { useAuth } from '../../contexts/AuthContext';

/** The slide-over panel: open or not, and which conversation it shows. */
interface PanelState {
  open: boolean;
  conversationId: string | null;
  /** A new conversation started from an entry point: its context and the question to edit. */
  seed: ChatSeed | null;
  setOpen: (open: boolean) => void;
  toggle: () => void;
  setConversationId: (id: string | null) => void;
  /** Open the panel on a new conversation with this context and a prefilled (unsent) question. */
  startConversation: (context: ChatContext, draft: string) => void;
}

export interface ChatSeed {
  key: number;
  context: ChatContext;
  draft: string;
}

let seedKey = 0;

export const useAssistantPanel = create<PanelState>((set) => ({
  open: false,
  conversationId: null,
  seed: null,
  setOpen: (open) => set({ open }),
  toggle: () => set((state) => ({ open: !state.open })),
  setConversationId: (conversationId) => set({ conversationId, seed: null }),
  startConversation: (context, draft) =>
    set({ open: true, conversationId: null, seed: { key: ++seedKey, context, draft } }),
}));

export const ASSISTANT_STATUS_KEY = ['assistant', 'status'] as const;
export const ASSISTANT_CONVERSATIONS_KEY = ['assistant', 'conversations'] as const;

/**
 * Whether the chat is offered to this user: they have `assistant.use`, a
 * key is stored and readable, and the owner has switched it on.
 */
export function useAssistantAvailability() {
  const { isAuthenticated, hasPermission } = useAuth();
  const canUse = isAuthenticated && hasPermission('assistant.use');
  const query = useQuery({
    queryKey: ASSISTANT_STATUS_KEY,
    queryFn: assistantApi.getStatus,
    enabled: canUse,
    staleTime: 60_000,
  });
  const status = query.data;
  const available = canUse && !!status && status.configured && !status.key_error && status.enabled;
  return {
    canUse,
    available,
    loading: canUse && query.isPending,
    paused: available && !!status?.paused,
    status,
  };
}
