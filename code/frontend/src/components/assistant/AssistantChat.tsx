import { useCallback, useEffect, useRef, useState, type FormEvent, type KeyboardEvent } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import { Loader2, RotateCcw, Send, Square } from 'lucide-react';
import {
  assistantChatApi,
  AssistantRequestError,
  type AssistantSource,
  type ChatContext,
  type ChatEvent,
  type ChatMessageStatus,
} from '../../api/assistant';
import { MarkdownLite } from './MarkdownLite';
import { ASSISTANT_CONVERSATIONS_KEY, ASSISTANT_STATUS_KEY } from './assistantState';

/** A message as shown in the chat (local ones have negative ids). */
export interface ShownMessage {
  id: number;
  role: 'user' | 'assistant';
  text: string;
  sources: AssistantSource[];
  status: ChatMessageStatus;
  error?: string;
  /** "Looking at …" while a tool runs. */
  statusLine?: string;
}

export const BUDGET_PAUSED_TEXT =
  "The assistant's monthly budget is used up. It resumes next month, or when an administrator raises the budget.";

const focusRing = 'focus:outline-none focus-visible:ring-2 focus-visible:ring-blue-500';

let localId = -1;
const nextLocalId = () => localId--;

interface Props {
  conversationId: string | null;
  /** Called when the first question creates a conversation. */
  onConversationCreated: (id: string) => void;
  context?: ChatContext;
  paused?: boolean;
  /** Narrow layout for the slide-over panel. */
  compact?: boolean;
}

function Sources({ items }: { items: AssistantSource[] }) {
  if (!items.length) return null;
  return (
    <div className="mt-2 flex flex-wrap items-center gap-1.5 text-xs text-gray-500 dark:text-gray-400" aria-label="Sources">
      <span>Sources:</span>
      {items.map((s) => (
        <span
          key={`${s.tool}:${s.target ?? ''}`}
          className="rounded-full bg-gray-100 dark:bg-gray-700/70 px-2 py-0.5 text-gray-600 dark:text-gray-300"
        >
          {s.label}
        </span>
      ))}
    </div>
  );
}

function AssistantBubble({ message, onAskAgain }: { message: ShownMessage; onAskAgain?: () => void }) {
  const streaming = message.status === 'streaming';
  return (
    <div className="max-w-full rounded-2xl rounded-tl-sm bg-white dark:bg-gray-800 px-3.5 py-2.5 text-sm text-gray-800 dark:text-gray-100 ring-1 ring-gray-200 dark:ring-gray-700">
      {message.text && <MarkdownLite text={message.text} />}
      {streaming && (
        <div role="status" className="mt-1 flex items-center gap-2 text-xs text-gray-500 dark:text-gray-400">
          <Loader2 size={14} className="animate-spin" aria-hidden />
          <span>{message.statusLine ?? (message.text ? 'Writing…' : 'Thinking…')}</span>
        </div>
      )}
      {message.status === 'interrupted' && (
        <p className="mt-2 text-xs text-amber-700 dark:text-amber-400">
          Interrupted —{' '}
          {onAskAgain ? (
            <button type="button" onClick={onAskAgain} className={`inline-flex items-center gap-1 font-medium underline ${focusRing}`}>
              <RotateCcw size={12} aria-hidden />
              ask again
            </button>
          ) : (
            'ask again'
          )}
        </p>
      )}
      {message.status === 'error' && (
        <p role="alert" className="mt-1 text-xs text-red-600 dark:text-red-400">
          {message.error ?? 'The assistant could not answer.'}
        </p>
      )}
      <Sources items={message.sources} />
    </div>
  );
}

export function AssistantChat({ conversationId, onConversationCreated, context, paused = false, compact = false }: Props) {
  const queryClient = useQueryClient();
  const [messages, setMessages] = useState<ShownMessage[]>([]);
  const [draft, setDraft] = useState('');
  const [streaming, setStreaming] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);
  const abortRef = useRef<AbortController | null>(null);
  /** The conversation this component created itself (already in state). */
  const createdRef = useRef<string | null>(null);
  const endRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (createdRef.current === conversationId && conversationId) return;
    // Switching conversation stops an answer still streaming for the old one.
    abortRef.current?.abort();
    createdRef.current = null;
    if (!conversationId) {
      setMessages([]);
      setLoadError(null);
      return;
    }
    let cancelled = false;
    setLoadError(null);
    assistantChatApi
      .getConversation(conversationId)
      .then((detail) => {
        if (!cancelled) setMessages(detail.messages.map((m) => ({ ...m })));
      })
      .catch(() => {
        if (!cancelled) setLoadError('Could not load this conversation.');
      });
    return () => {
      cancelled = true;
    };
  }, [conversationId]);

  useEffect(() => () => abortRef.current?.abort(), []);

  useEffect(() => {
    endRef.current?.scrollIntoView?.({ block: 'end' });
  }, [messages]);

  const update = useCallback((id: number, change: (m: ShownMessage) => ShownMessage) => {
    setMessages((prev) => prev.map((m) => (m.id === id ? change(m) : m)));
  }, []);

  const ask = useCallback(
    async (question: string) => {
      const text = question.trim();
      if (!text || streaming || paused) return;
      setStreaming(true);
      const userId = nextLocalId();
      const answerId = nextLocalId();
      setMessages((prev) => [
        ...prev,
        { id: userId, role: 'user', text, sources: [], status: 'complete' },
        { id: answerId, role: 'assistant', text: '', sources: [], status: 'streaming' },
      ]);
      let finished = false;
      const onEvent = (event: ChatEvent) => {
        switch (event.event) {
          case 'status':
            update(answerId, (m) => ({ ...m, statusLine: event.text }));
            break;
          case 'text_delta':
            update(answerId, (m) => ({ ...m, text: m.text + event.text, statusLine: undefined }));
            break;
          case 'sources':
            update(answerId, (m) => ({ ...m, sources: event.items }));
            break;
          case 'done':
            finished = true;
            update(answerId, (m) => ({ ...m, id: event.message_id, status: 'complete', statusLine: undefined }));
            break;
          case 'error':
            finished = true;
            update(answerId, (m) => ({ ...m, status: 'error', error: event.message, statusLine: undefined }));
            break;
        }
      };
      const controller = new AbortController();
      abortRef.current = controller;
      try {
        let id = conversationId;
        if (!id) {
          const created = await assistantChatApi.createConversation(context);
          id = created.id;
          createdRef.current = id;
          onConversationCreated(id);
        }
        await assistantChatApi.ask(id, text, context, onEvent, controller.signal);
        if (!finished) update(answerId, (m) => ({ ...m, status: 'interrupted', statusLine: undefined }));
      } catch (e) {
        if (e instanceof AssistantRequestError) {
          update(answerId, (m) => ({ ...m, status: 'error', error: e.message, statusLine: undefined }));
          if (e.status === 503) void queryClient.invalidateQueries({ queryKey: ASSISTANT_STATUS_KEY });
        } else if (!finished) {
          update(answerId, (m) => ({ ...m, status: 'interrupted', statusLine: undefined }));
        }
      } finally {
        abortRef.current = null;
        setStreaming(false);
        void queryClient.invalidateQueries({ queryKey: ASSISTANT_CONVERSATIONS_KEY });
      }
    },
    [conversationId, context, onConversationCreated, paused, queryClient, streaming, update],
  );

  const submit = (e?: FormEvent) => {
    e?.preventDefault();
    const text = draft;
    setDraft('');
    void ask(text);
  };

  const onKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === 'Enter' && !e.shiftKey && !e.nativeEvent.isComposing) {
      e.preventDefault();
      submit();
    }
  };

  /** The question before an assistant message, for "ask again". */
  const questionBefore = (index: number): string | undefined => {
    for (let i = index - 1; i >= 0; i--) if (messages[i].role === 'user') return messages[i].text;
    return undefined;
  };

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className={`flex-1 min-h-0 overflow-y-auto ${compact ? 'px-3' : 'px-4'} py-4 space-y-3`} aria-live="polite">
        {loadError && <p role="alert" className="text-sm text-red-600 dark:text-red-400">{loadError}</p>}
        {!messages.length && !loadError && (
          <div className="mt-6 text-center text-sm text-gray-500 dark:text-gray-400">
            <p className="font-medium text-gray-700 dark:text-gray-200">Ask about your apps and server</p>
            <p className="mt-1">For example: “Why is Jellyfin unhealthy?” or “What changed last night?”</p>
            <p className="mt-3 text-xs">The assistant can look things up but never changes anything.</p>
          </div>
        )}
        {messages.map((m, index) =>
          m.role === 'user' ? (
            <div key={m.id} className="flex justify-end">
              <div className="max-w-[85%] whitespace-pre-wrap break-words rounded-2xl rounded-tr-sm bg-blue-600 px-3.5 py-2 text-sm text-white">
                {m.text}
              </div>
            </div>
          ) : (
            <div key={m.id} className="flex">
              <AssistantBubble
                message={m}
                onAskAgain={
                  m.status === 'interrupted' && !streaming && !paused && questionBefore(index)
                    ? () => void ask(questionBefore(index) as string)
                    : undefined
                }
              />
            </div>
          ),
        )}
        <div ref={endRef} />
      </div>
      {paused && (
        <p role="alert" className="mx-3 mb-2 rounded-md bg-amber-50 dark:bg-amber-900/30 px-3 py-2 text-xs text-amber-800 dark:text-amber-300">
          {BUDGET_PAUSED_TEXT}
        </p>
      )}
      <form onSubmit={submit} className="flex items-end gap-2 border-t border-gray-200 dark:border-gray-700 p-3">
        <label className="sr-only" htmlFor={compact ? 'assistant-input-panel' : 'assistant-input-page'}>
          Ask the assistant
        </label>
        <textarea
          id={compact ? 'assistant-input-panel' : 'assistant-input-page'}
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={onKeyDown}
          rows={compact ? 2 : 3}
          maxLength={8000}
          disabled={paused}
          placeholder={paused ? 'Paused' : 'Ask a question…'}
          className={`flex-1 resize-none rounded-lg border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900 px-3 py-2 text-sm text-gray-900 dark:text-white placeholder:text-gray-400 disabled:opacity-60 ${focusRing}`}
        />
        {streaming ? (
          <button
            type="button"
            onClick={() => abortRef.current?.abort()}
            aria-label="Stop"
            title="Stop"
            className={`flex h-9 w-9 items-center justify-center rounded-lg bg-gray-200 dark:bg-gray-700 text-gray-700 dark:text-gray-200 hover:bg-gray-300 dark:hover:bg-gray-600 ${focusRing}`}
          >
            <Square size={14} fill="currentColor" />
          </button>
        ) : (
          <button
            type="submit"
            aria-label="Send"
            title="Send (Enter)"
            disabled={paused || !draft.trim()}
            className={`flex h-9 w-9 items-center justify-center rounded-lg bg-blue-600 text-white hover:bg-blue-700 disabled:opacity-40 ${focusRing}`}
          >
            <Send size={16} />
          </button>
        )}
      </form>
    </div>
  );
}
