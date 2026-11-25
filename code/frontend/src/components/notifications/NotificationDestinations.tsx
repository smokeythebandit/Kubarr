import { useCallback, useEffect, useRef, useState } from 'react';
import { Bell, Loader2 } from 'lucide-react';
import { notificationsApi, type UserNotificationPref } from '../../api/notifications';
import { errorDetail, needsNewCode, normalizeCodeInput, retryAfterSeconds } from './verificationErrors';

// The signed-in user's own notification destinations. External channels only
// deliver to a destination after the user confirms a code sent to it.

const CHANNELS: Record<string, { label: string; field: string; placeholder: string; type: string }> = {
  email: { label: 'Email', field: 'Email address', placeholder: 'you@example.com', type: 'email' },
  telegram: { label: 'Telegram', field: 'Chat ID', placeholder: '123456789', type: 'text' },
  messagebird: { label: 'SMS', field: 'Phone number', placeholder: '+31612345678', type: 'tel' },
};

type Chip = { label: string; className: string };

function chipFor(pref: UserNotificationPref): Chip {
  if (pref.verified) {
    return { label: 'Verified', className: 'text-emerald-700 bg-emerald-50 dark:text-emerald-300 dark:bg-emerald-900/40' };
  }
  if (pref.verification_pending) {
    return { label: 'Code sent', className: 'text-blue-700 bg-blue-50 dark:text-blue-300 dark:bg-blue-900/40' };
  }
  return { label: 'Not verified', className: 'text-amber-700 bg-amber-50 dark:text-amber-300 dark:bg-amber-900/40' };
}

/** A 1 s countdown; `start(n)` restarts it. */
function useCountdown(): [number, (secs: number) => void] {
  const [left, setLeft] = useState(0);
  useEffect(() => {
    if (left <= 0) return undefined;
    const timer = setTimeout(() => setLeft((s) => Math.max(0, s - 1)), 1000);
    return () => clearTimeout(timer);
  }, [left]);
  return [left, setLeft];
}

export function DestinationRow({ initial }: { initial: UserNotificationPref }) {
  const meta = CHANNELS[initial.channel_type] ?? {
    label: initial.channel_type,
    field: 'Destination',
    placeholder: '',
    type: 'text',
  };
  const [pref, setPref] = useState(initial);
  const [destination, setDestination] = useState('');
  const [code, setCode] = useState('');
  const [sentTo, setSentTo] = useState<string | null>(null);
  const [busy, setBusy] = useState<'save' | 'toggle' | 'send' | 'confirm' | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [cooldown, startCooldown] = useCountdown();
  const [focusCode, setFocusCode] = useState(false);
  const codeRef = useRef<HTMLInputElement>(null);
  const id = `dest-${pref.channel_type}`;

  const resetVerification = () => {
    setCode('');
    setSentTo(null);
  };

  const save = async () => {
    const value = destination.trim();
    if (!value) return;
    setBusy('save');
    setError(null);
    try {
      const updated = await notificationsApi.updatePreference(pref.channel_type, {
        enabled: pref.enabled,
        destination: value,
      });
      // The server clears the pending code only when the destination changed.
      if (!updated.verification_pending) resetVerification();
      setPref(updated);
      setDestination('');
    } catch (err) {
      setError(errorDetail(err, 'Could not save the destination'));
    } finally {
      setBusy(null);
    }
  };

  const toggle = async () => {
    setBusy('toggle');
    setError(null);
    try {
      setPref(await notificationsApi.updatePreference(pref.channel_type, { enabled: !pref.enabled }));
    } catch (err) {
      setError(errorDetail(err, 'Could not update the channel'));
    } finally {
      setBusy(null);
    }
  };

  const sendCode = async () => {
    setBusy('send');
    setError(null);
    try {
      const res = await notificationsApi.startVerification(pref.channel_type);
      setSentTo(res.destination);
      setPref((p) => ({ ...p, verification_pending: true }));
      startCooldown(res.resend_after_seconds);
      setFocusCode(true);
    } catch (err) {
      const wait = retryAfterSeconds(err);
      if (wait) startCooldown(wait);
      setError(errorDetail(err, 'Could not send the code'));
    } finally {
      setBusy(null);
    }
  };

  const confirm = async (e: React.FormEvent) => {
    e.preventDefault();
    if (code.length !== 6) return;
    setBusy('confirm');
    setError(null);
    try {
      setPref(await notificationsApi.confirmVerification(pref.channel_type, code));
      resetVerification();
    } catch (err) {
      setError(errorDetail(err, 'Could not confirm the code'));
      if (needsNewCode(err)) {
        // The pending code is gone server-side; resync so the form closes.
        resetVerification();
        try {
          const fresh = (await notificationsApi.getPreferences()).find((p) => p.channel_type === pref.channel_type);
          setPref((p) => fresh ?? { ...p, verification_pending: false });
        } catch {
          setPref((p) => ({ ...p, verification_pending: false }));
        }
      }
    } finally {
      setBusy(null);
    }
  };

  const chip = chipFor(pref);
  const hasDestination = !!pref.destination;
  const canSend = pref.enabled && hasDestination && !pref.verified && cooldown === 0 && busy === null;
  const showCodeForm = pref.enabled && !pref.verified && (pref.verification_pending || sentTo !== null);

  // The code input mounts after a successful send; move focus to it then.
  useEffect(() => {
    if (focusCode && showCodeForm) {
      codeRef.current?.focus();
      setFocusCode(false);
    }
  }, [focusCode, showCodeForm]);

  return (
    <div className="rounded-lg border border-gray-200 dark:border-gray-700 bg-gray-50 dark:bg-gray-900 p-4 space-y-3" data-testid={id}>
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="flex items-center gap-2">
          <span className="font-medium text-gray-900 dark:text-white">{meta.label}</span>
          {hasDestination && (
            <span className={`rounded-full px-2 py-0.5 text-xs font-semibold ${chip.className}`}>{chip.label}</span>
          )}
        </div>
        <button
          type="button"
          role="switch"
          aria-checked={pref.enabled}
          aria-label={`${meta.label} notifications`}
          disabled={busy !== null}
          onClick={toggle}
          className={`relative inline-flex h-6 w-11 shrink-0 items-center rounded-full transition-colors focus:outline-none focus-visible:ring-2 focus-visible:ring-blue-500 disabled:opacity-50 ${
            pref.enabled ? 'bg-blue-600' : 'bg-gray-300 dark:bg-gray-600'
          }`}
        >
          <span
            className={`inline-block h-4 w-4 transform rounded-full bg-white transition-transform ${
              pref.enabled ? 'translate-x-6' : 'translate-x-1'
            }`}
          />
        </button>
      </div>

      <div className="flex flex-wrap items-end gap-2">
        <label htmlFor={`${id}-input`} className="grid flex-1 min-w-[12rem] gap-1 text-sm font-medium text-gray-700 dark:text-gray-300">
          {meta.field}
          <input
            id={`${id}-input`}
            type={meta.type}
            value={destination}
            onChange={(e) => setDestination(e.target.value)}
            placeholder={pref.destination ? `Current: ${pref.destination}` : meta.placeholder}
            className="rounded-md border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-800 px-3 py-2 text-sm text-gray-900 dark:text-white"
          />
        </label>
        <button
          type="button"
          onClick={save}
          disabled={busy !== null || !destination.trim()}
          className="rounded-md border border-gray-300 dark:border-gray-600 px-3 py-2 text-sm font-medium text-gray-700 dark:text-gray-200 hover:bg-gray-100 dark:hover:bg-gray-700 disabled:opacity-50"
        >
          {busy === 'save' ? (
            <>
              <Loader2 size={16} className="animate-spin" aria-hidden="true" />
              <span className="sr-only">Saving</span>
            </>
          ) : (
            'Save'
          )}
        </button>
      </div>

      {pref.enabled && hasDestination && !pref.verified && (
        <p className="text-xs text-amber-700 dark:text-amber-300">Kubarr only sends here after you confirm it.</p>
      )}

      {hasDestination && !pref.verified && (
        <div className="flex flex-wrap items-center gap-2">
          <button
            type="button"
            onClick={sendCode}
            disabled={!canSend}
            className="rounded-md bg-blue-600 px-3 py-2 text-sm font-medium text-white hover:bg-blue-700 disabled:opacity-50"
          >
            {busy === 'send' ? 'Sending…' : cooldown > 0 ? `Send code (${cooldown}s)` : pref.verification_pending ? 'Send new code' : 'Send code'}
          </button>
          {sentTo && (
            <span className="text-sm text-gray-600 dark:text-gray-400" aria-hidden="true">
              Code sent to {sentTo}
            </span>
          )}
        </div>
      )}

      {/* Always mounted (visually hidden) so screen readers announce new text. */}
      <p role="status" aria-live="polite" className="sr-only">
        {sentTo && !pref.verified ? `Code sent to ${sentTo}` : ''}
      </p>

      {showCodeForm && (
        <form onSubmit={confirm} className="flex flex-wrap items-end gap-2">
          <label htmlFor={`${id}-code`} className="grid gap-1 text-sm font-medium text-gray-700 dark:text-gray-300">
            Verification code
            <input
              id={`${id}-code`}
              ref={codeRef}
              value={code}
              onChange={(e) => setCode(normalizeCodeInput(e.target.value))}
              onPaste={(e) => {
                e.preventDefault();
                setCode(normalizeCodeInput(e.clipboardData.getData('text')));
              }}
              inputMode="numeric"
              autoComplete="one-time-code"
              placeholder="123456"
              className="w-32 rounded-md border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-800 px-3 py-2 text-sm tracking-widest text-gray-900 dark:text-white"
            />
          </label>
          <button
            type="submit"
            disabled={busy !== null || code.length !== 6}
            className="rounded-md bg-emerald-600 px-3 py-2 text-sm font-medium text-white hover:bg-emerald-700 disabled:opacity-50"
          >
            {busy === 'confirm' ? 'Confirming…' : 'Confirm'}
          </button>
        </form>
      )}

      {error && (
        <p role="alert" className="text-sm text-red-600 dark:text-red-400">
          {error}
        </p>
      )}
    </div>
  );
}

export default function NotificationDestinations() {
  const [prefs, setPrefs] = useState<UserNotificationPref[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      setPrefs(await notificationsApi.getPreferences());
    } catch (err) {
      setError(errorDetail(err, 'Could not load notification destinations'));
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  return (
    <div className="bg-white dark:bg-gray-800 rounded-lg border border-gray-200 dark:border-gray-700 p-6">
      <h3 className="text-lg font-semibold text-gray-900 dark:text-white mb-1 flex items-center gap-2">
        <Bell size={20} className="text-blue-500" />
        Notification destinations
      </h3>
      <p className="text-sm text-gray-500 dark:text-gray-400 mb-4">
        Where Kubarr may reach you outside the app. Confirm each destination with the code we send to it.
      </p>
      {error && (
        <p role="alert" className="text-sm text-red-600 dark:text-red-400">
          {error}
        </p>
      )}
      {!prefs && !error && (
        <div className="flex items-center gap-2 text-gray-500">
          <Loader2 size={16} className="animate-spin" />
          Loading...
        </div>
      )}
      {prefs && (
        <div className="space-y-3">
          {prefs.map((p) => (
            <DestinationRow key={p.channel_type} initial={p} />
          ))}
        </div>
      )}
    </div>
  );
}
