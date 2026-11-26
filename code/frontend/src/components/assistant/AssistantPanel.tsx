import { useEffect, useRef } from 'react';
import { Link, useLocation } from 'react-router-dom';
import { Maximize2, Plus, Sparkles, X } from 'lucide-react';
import { AssistantChat } from './AssistantChat';
import type { ChatContext } from '../../api/assistant';
import { useAssistantAvailability, useAssistantPanel } from './assistantState';

const focusRing = 'focus:outline-none focus-visible:ring-2 focus-visible:ring-blue-500';

/** Ctrl+J / ⌘J toggles the panel. */
function isAssistantShortcut(e: KeyboardEvent): boolean {
  return (e.ctrlKey || e.metaKey) && !e.altKey && !e.shiftKey && e.key.toLowerCase() === 'j';
}

/** Top-bar button. Hidden unless the assistant is offered to this user. */
export function AssistantButton({ className = '' }: { className?: string }) {
  const { available } = useAssistantAvailability();
  const { open, toggle } = useAssistantPanel();
  if (!available) return null;
  return (
    <button
      type="button"
      onClick={toggle}
      aria-label="Assistant"
      aria-expanded={open}
      title="Assistant (Ctrl+J)"
      className={`flex items-center justify-center w-9 h-9 rounded-md text-gray-600 dark:text-gray-300 hover:bg-gray-200 dark:hover:bg-gray-700 transition-colors ${focusRing} ${className}`}
    >
      <Sparkles size={18} strokeWidth={2} />
    </button>
  );
}

/** The slide-over chat, rendered once for the whole app. */
export function AssistantPanel() {
  const { available, paused } = useAssistantAvailability();
  const { open, setOpen, toggle, conversationId, setConversationId, seed } = useAssistantPanel();
  const location = useLocation();
  const panelRef = useRef<HTMLDivElement>(null);
  const onAssistantPage = location.pathname === '/assistant';

  useEffect(() => {
    if (!available) return;
    const onKey = (e: KeyboardEvent) => {
      if (isAssistantShortcut(e)) {
        e.preventDefault();
        toggle();
      } else if (e.key === 'Escape' && useAssistantPanel.getState().open) {
        setOpen(false);
      }
    };
    document.addEventListener('keydown', onKey);
    return () => document.removeEventListener('keydown', onKey);
  }, [available, toggle, setOpen]);

  useEffect(() => {
    if (open) panelRef.current?.querySelector('textarea')?.focus();
  }, [open, seed?.key]);

  if (!available || !open || onAssistantPage) return null;

  return (
    <div
      ref={panelRef}
      role="dialog"
      aria-label="Assistant"
      className="fixed inset-y-0 right-0 z-50 flex w-full flex-col border-l border-gray-200 dark:border-gray-700 bg-gray-50 dark:bg-gray-900 shadow-2xl sm:w-[28rem]"
    >
      <div className="flex items-center gap-2 border-b border-gray-200 dark:border-gray-700 bg-white dark:bg-gray-800 px-3 py-2.5">
        <Sparkles size={18} className="text-blue-600 dark:text-blue-400" aria-hidden />
        <h2 className="flex-1 text-sm font-semibold text-gray-900 dark:text-white">Assistant</h2>
        <button
          type="button"
          onClick={() => setConversationId(null)}
          aria-label="New conversation"
          title="New conversation"
          className={`rounded-md p-1.5 text-gray-500 hover:bg-gray-100 dark:hover:bg-gray-700 ${focusRing}`}
        >
          <Plus size={16} />
        </button>
        <Link
          to={conversationId ? `/assistant?c=${encodeURIComponent(conversationId)}` : '/assistant'}
          onClick={() => setOpen(false)}
          aria-label="Open the assistant page"
          title="All conversations"
          className={`rounded-md p-1.5 text-gray-500 hover:bg-gray-100 dark:hover:bg-gray-700 ${focusRing}`}
        >
          <Maximize2 size={16} />
        </Link>
        <button
          type="button"
          onClick={() => setOpen(false)}
          aria-label="Close assistant"
          title="Close (Esc)"
          className={`rounded-md p-1.5 text-gray-500 hover:bg-gray-100 dark:hover:bg-gray-700 ${focusRing}`}
        >
          <X size={16} />
        </button>
      </div>
      <div className="min-h-0 flex-1">
        <AssistantChat
          conversationId={conversationId}
          onConversationCreated={setConversationId}
          context={{ page: location.pathname, ...seed?.context }}
          seed={seed ? { key: seed.key, draft: seed.draft } : undefined}
          paused={paused}
          compact
        />
      </div>
    </div>
  );
}

/**
 * A button that opens the panel on a new conversation with a prefilled
 * question the person edits and sends. Hidden unless the assistant is
 * offered; never sends anything itself.
 */
export function AskAssistantButton({
  context,
  draft,
  label,
  resolveContext,
  className = '',
}: {
  context: ChatContext;
  draft: string;
  label: string;
  /** Refine the context when clicked (e.g. look up an operation). */
  resolveContext?: () => Promise<ChatContext>;
  className?: string;
}) {
  const { available } = useAssistantAvailability();
  const startConversation = useAssistantPanel((s) => s.startConversation);
  if (!available) return null;
  const onClick = async () => {
    let chosen = context;
    if (resolveContext) {
      try {
        chosen = await resolveContext();
      } catch {
        chosen = context;
      }
    }
    startConversation(chosen, draft);
  };
  return (
    <button
      type="button"
      onClick={() => void onClick()}
      className={`inline-flex items-center gap-1.5 ${focusRing} ${className}`}
    >
      <Sparkles size={14} aria-hidden />
      {label}
    </button>
  );
}
