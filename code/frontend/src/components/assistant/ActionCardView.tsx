import { useEffect, useState } from 'react';
import { Link } from 'react-router-dom';
import { CheckCircle2, Clock, Loader2, Play, XCircle, Zap } from 'lucide-react';
import { assistantChatApi, type ActionCard } from '../../api/assistant';

const focusRing = 'focus:outline-none focus-visible:ring-2 focus-visible:ring-blue-500';

/** "4:59" until the action expires. */
export function formatRemaining(ms: number): string {
  const total = Math.max(0, Math.ceil(ms / 1000));
  const minutes = Math.floor(total / 60);
  const seconds = total % 60;
  return `${minutes}:${seconds.toString().padStart(2, '0')}`;
}

function errorText(e: unknown, fallback: string): string {
  const detail = (e as { response?: { data?: { detail?: unknown } } })?.response?.data?.detail;
  return typeof detail === 'string' ? detail : fallback;
}

interface Props {
  card: ActionCard;
  /** The card changed (after Run or Cancel); the parent stores the new state. */
  onChange: (card: ActionCard) => void;
  /** For tests: the current time. */
  now?: () => number;
}

/**
 * A proposed action: exactly what will happen, Run / Cancel while it is
 * pending (with the time left), then the outcome.
 */
export function ActionCardView({ card, onChange, now = Date.now }: Props) {
  const [busy, setBusy] = useState<'run' | 'cancel' | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [clock, setClock] = useState(now);
  const expiresAt = Date.parse(card.expires_at);
  const expired = card.status === 'expired' || (card.status === 'pending' && clock >= expiresAt);
  const status = expired && card.status === 'pending' ? 'expired' : card.status;
  const pending = status === 'pending';

  useEffect(() => {
    if (!pending) return;
    const timer = setInterval(() => setClock(now()), 1000);
    return () => clearInterval(timer);
  }, [pending, now]);

  const run = async () => {
    if (busy) return;
    setBusy('run');
    setError(null);
    try {
      const result = await assistantChatApi.runAction(card.id);
      onChange({ ...card, status: result.status, operation_id: result.operation_id });
    } catch (e) {
      const message = errorText(e, 'The action could not be run.');
      setError(message);
      // Refused at run time: the action is used up (failed, expired or already run).
      onChange({ ...card, status: 'failed', error: message });
    } finally {
      setBusy(null);
    }
  };

  const cancel = async () => {
    if (busy) return;
    setBusy('cancel');
    setError(null);
    try {
      await assistantChatApi.cancelAction(card.id);
      onChange({ ...card, status: 'cancelled' });
    } catch (e) {
      setError(errorText(e, 'The action could not be cancelled.'));
    } finally {
      setBusy(null);
    }
  };

  const { summary } = card;
  return (
    <div
      className="mt-2 rounded-xl border border-blue-200 dark:border-blue-900 bg-blue-50/60 dark:bg-blue-950/30 p-3 text-sm"
      aria-label={`Proposed action: ${summary.title}`}
      data-status={status}
    >
      <div className="flex items-start gap-2">
        <Zap size={16} className="mt-0.5 shrink-0 text-blue-600 dark:text-blue-400" aria-hidden />
        <div className="min-w-0 flex-1">
          <p className="font-medium text-gray-900 dark:text-white">{summary.title}</p>
          <p className="mt-0.5 text-gray-700 dark:text-gray-300">{summary.effect}</p>
          <dl className="mt-1.5 flex flex-wrap gap-x-4 gap-y-0.5 text-xs text-gray-600 dark:text-gray-400">
            <div className="flex gap-1">
              <dt>App:</dt>
              <dd>
                <Link to="/apps" className="font-medium text-blue-700 dark:text-blue-300 hover:underline">
                  {card.app}
                </Link>
              </dd>
            </div>
            {(summary.from_version || summary.to_version) && (
              <div className="flex gap-1">
                <dt>Version:</dt>
                <dd className="font-mono">
                  {summary.from_version && summary.to_version
                    ? `${summary.from_version} → ${summary.to_version}`
                    : summary.to_version ?? summary.from_version}
                </dd>
              </div>
            )}
          </dl>
        </div>
      </div>

      {pending && (
        <div className="mt-3 flex flex-wrap items-center gap-2">
          <button
            type="button"
            onClick={() => void run()}
            disabled={busy !== null}
            className={`inline-flex items-center gap-1.5 rounded-lg bg-blue-600 px-3 py-1.5 text-xs font-semibold text-white hover:bg-blue-700 disabled:opacity-50 ${focusRing}`}
          >
            {busy === 'run' ? <Loader2 size={14} className="animate-spin" aria-hidden /> : <Play size={14} aria-hidden />}
            Run
          </button>
          <button
            type="button"
            onClick={() => void cancel()}
            disabled={busy !== null}
            className={`rounded-lg px-3 py-1.5 text-xs font-medium text-gray-700 dark:text-gray-200 ring-1 ring-gray-300 dark:ring-gray-600 hover:bg-gray-100 dark:hover:bg-gray-800 disabled:opacity-50 ${focusRing}`}
          >
            Cancel
          </button>
          <span className="ml-auto inline-flex items-center gap-1 text-xs text-gray-500 dark:text-gray-400">
            <Clock size={12} aria-hidden />
            Expires in {formatRemaining(expiresAt - clock)}
          </span>
        </div>
      )}

      {!pending && (
        <p className="mt-2 flex items-center gap-1.5 text-xs" role="status">
          {status === 'running' && (
            <>
              <Loader2 size={14} className="animate-spin text-blue-600" aria-hidden />
              <span className="text-gray-700 dark:text-gray-300">
                Queued{card.operation_id ? ' as an operation' : ''}.{' '}
                <Link to="/apps?tab=operations" className="font-medium text-blue-700 dark:text-blue-300 underline">
                  View operations
                </Link>
              </span>
            </>
          )}
          {status === 'done' && (
            <>
              <CheckCircle2 size={14} className="text-green-600" aria-hidden />
              <span className="text-gray-700 dark:text-gray-300">
                Done.
                {card.operation_id && (
                  <>
                    {' '}
                    <Link to="/apps?tab=operations" className="font-medium text-blue-700 dark:text-blue-300 underline">
                      View operations
                    </Link>
                  </>
                )}
              </span>
            </>
          )}
          {status === 'failed' && (
            <>
              <XCircle size={14} className="text-red-600" aria-hidden />
              <span className="text-red-700 dark:text-red-400">Did not complete{card.error ? `: ${card.error}` : '.'}</span>
            </>
          )}
          {status === 'cancelled' && <span className="text-gray-500 dark:text-gray-400">Cancelled.</span>}
          {status === 'expired' && (
            <span className="text-gray-500 dark:text-gray-400">Expired. Ask the assistant again if you still want this.</span>
          )}
        </p>
      )}

      {error && status !== 'failed' && (
        <p role="alert" className="mt-2 text-xs text-red-600 dark:text-red-400">
          {error}
        </p>
      )}
    </div>
  );
}
