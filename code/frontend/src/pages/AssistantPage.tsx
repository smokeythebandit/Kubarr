import { useState } from 'react';
import { useSearchParams } from 'react-router-dom';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { formatDistanceToNow } from 'date-fns';
import { MessageSquare, Plus, Sparkles, Trash2 } from 'lucide-react';
import { assistantChatApi, type ConversationSummary } from '../api/assistant';
import { AssistantChat } from '../components/assistant/AssistantChat';
import { ASSISTANT_CONVERSATIONS_KEY, useAssistantAvailability } from '../components/assistant/assistantState';
import { useAuth } from '../contexts/AuthContext';

const focusRing = 'focus:outline-none focus-visible:ring-2 focus-visible:ring-blue-500';

function Unavailable({ canUse }: { canUse: boolean }) {
  const { hasPermission } = useAuth();
  const message = !canUse
    ? 'You do not have access to the assistant. An administrator can grant the "Chat with the AI assistant" permission.'
    : hasPermission('settings.manage')
      ? 'The assistant is not set up or is switched off. Add an Anthropic API key and switch it on in Settings → Assistant.'
      : 'The assistant is not available right now. Ask an administrator to set it up.';
  return (
    <div className="mx-auto max-w-xl rounded-xl border border-gray-200 dark:border-gray-700 bg-white dark:bg-gray-800 p-6 text-center">
      <Sparkles className="mx-auto mb-2 text-gray-400" aria-hidden />
      <p className="text-sm text-gray-600 dark:text-gray-300">{message}</p>
    </div>
  );
}

function ConversationRow({
  conversation,
  selected,
  onSelect,
  onDelete,
}: {
  conversation: ConversationSummary;
  selected: boolean;
  onSelect: () => void;
  onDelete: () => Promise<void>;
}) {
  const [confirming, setConfirming] = useState(false);
  const [deleting, setDeleting] = useState(false);
  return (
    <li
      className={`group flex items-center gap-2 rounded-lg px-2.5 py-2 ${
        selected
          ? 'bg-white dark:bg-gray-800 ring-1 ring-inset ring-gray-200 dark:ring-gray-700'
          : 'hover:bg-gray-100 dark:hover:bg-gray-800/60'
      }`}
    >
      <button type="button" onClick={onSelect} aria-current={selected || undefined} className={`min-w-0 flex-1 text-left ${focusRing}`}>
        <span className="block truncate text-sm font-medium text-gray-900 dark:text-white">{conversation.title}</span>
        <span className="block text-xs text-gray-500 dark:text-gray-400">
          {formatDistanceToNow(new Date(conversation.updated_at), { addSuffix: true })}
        </span>
      </button>
      {confirming ? (
        <span className="flex items-center gap-1 text-xs">
          <button
            type="button"
            disabled={deleting}
            onClick={async () => {
              setDeleting(true);
              try {
                await onDelete();
              } finally {
                setDeleting(false);
                setConfirming(false);
              }
            }}
            className={`rounded px-1.5 py-0.5 font-medium text-red-600 hover:bg-red-50 dark:hover:bg-red-900/30 ${focusRing}`}
          >
            Delete
          </button>
          <button type="button" onClick={() => setConfirming(false)} className={`rounded px-1.5 py-0.5 text-gray-500 ${focusRing}`}>
            Keep
          </button>
        </span>
      ) : (
        <button
          type="button"
          onClick={() => setConfirming(true)}
          aria-label={`Delete conversation ${conversation.title}`}
          title="Delete conversation"
          className={`rounded p-1 text-gray-400 opacity-60 hover:text-red-600 group-hover:opacity-100 ${focusRing}`}
        >
          <Trash2 size={14} />
        </button>
      )}
    </li>
  );
}

export default function AssistantPage() {
  const { canUse, available, loading, paused } = useAssistantAvailability();
  const [params, setParams] = useSearchParams();
  const queryClient = useQueryClient();
  const selected = params.get('c');
  const [error, setError] = useState<string | null>(null);

  const conversations = useQuery({
    queryKey: ASSISTANT_CONVERSATIONS_KEY,
    queryFn: assistantChatApi.listConversations,
    enabled: available,
  });

  const select = (id: string | null) => {
    setParams(id ? { c: id } : {}, { replace: false });
  };

  const remove = async (id: string) => {
    setError(null);
    try {
      await assistantChatApi.deleteConversation(id);
      if (selected === id) select(null);
      await queryClient.invalidateQueries({ queryKey: ASSISTANT_CONVERSATIONS_KEY });
    } catch {
      setError('Could not delete the conversation.');
    }
  };

  if (loading) {
    return <p className="text-sm text-gray-500" role="status">Loading…</p>;
  }
  if (!available) return <Unavailable canUse={canUse} />;

  const list = conversations.data ?? [];

  return (
    <div className="mx-auto grid max-w-6xl gap-4 md:grid-cols-[16rem_1fr]" style={{ height: 'calc(100vh - 9rem)' }}>
      <aside className="flex min-h-0 flex-col">
        <div className="mb-3 flex items-center justify-between">
          <h1 className="flex items-center gap-2 text-lg font-semibold text-gray-900 dark:text-white">
            <Sparkles size={18} className="text-blue-600 dark:text-blue-400" aria-hidden />
            Assistant
          </h1>
          <button
            type="button"
            onClick={() => select(null)}
            className={`flex items-center gap-1 rounded-md px-2 py-1 text-sm text-blue-600 dark:text-blue-400 hover:bg-blue-50 dark:hover:bg-blue-900/30 ${focusRing}`}
          >
            <Plus size={14} /> New
          </button>
        </div>
        {error && <p role="alert" className="mb-2 text-xs text-red-600 dark:text-red-400">{error}</p>}
        {conversations.isError && <p role="alert" className="text-xs text-red-600">Could not load conversations.</p>}
        {!conversations.isPending && !list.length && (
          <p className="flex items-center gap-2 text-sm text-gray-500 dark:text-gray-400">
            <MessageSquare size={14} aria-hidden /> No conversations yet.
          </p>
        )}
        <ul className="min-h-0 flex-1 space-y-1 overflow-y-auto" aria-label="Conversations">
          {list.map((c) => (
            <ConversationRow
              key={c.id}
              conversation={c}
              selected={c.id === selected}
              onSelect={() => select(c.id)}
              onDelete={() => remove(c.id)}
            />
          ))}
        </ul>
        <p className="mt-3 text-xs text-gray-400">Conversations are deleted automatically after the retention period.</p>
      </aside>
      <section className="min-h-0 overflow-hidden rounded-xl border border-gray-200 dark:border-gray-700 bg-gray-50 dark:bg-gray-900/40">
        <AssistantChat
          conversationId={selected}
          onConversationCreated={(id) => select(id)}
          context={{ page: '/assistant' }}
          paused={paused}
        />
      </section>
    </div>
  );
}
