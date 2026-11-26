import { useEffect, useMemo, useState } from 'react';
import { Loader2, Moon } from 'lucide-react';
import { notificationsApi, type QuietHours } from '../../api/notifications';
import { errorDetail } from './verificationErrors';
import { QUIET_HOURS_DESCRIPTION, browserTimezone, minutesToTime, quietHoursPreview, timeToMinutes, timezoneNames } from './quietHoursModel';

// The signed-in user's own quiet hours: email, Telegram and SMS wait while
// they are on and arrive as one summary when they end. The bell is never held.

const focusRing = 'focus:outline-none focus-visible:ring-2 focus-visible:ring-blue-500';
const timeInput = `rounded-lg border border-gray-200 dark:border-gray-600 bg-white dark:bg-gray-900 px-2 py-1 font-mono text-sm text-gray-900 dark:text-white ${focusRing}`;

type Draft = Omit<QuietHours, 'configured'>;

function Switch({ checked, label, onChange, disabled }: { checked: boolean; label: string; onChange: () => void; disabled?: boolean }) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      onClick={onChange}
      disabled={disabled}
      className={`relative inline-flex h-6 w-11 shrink-0 items-center rounded-full transition-colors disabled:opacity-50 ${focusRing} ${
        checked ? 'bg-blue-600' : 'bg-gray-300 dark:bg-gray-600'
      }`}
    >
      <span className={`inline-block h-4 w-4 transform rounded-full bg-white transition-transform ${checked ? 'translate-x-6' : 'translate-x-1'}`} />
    </button>
  );
}

export function QuietHoursForm() {
  const [saved, setSaved] = useState<Draft | null>(null);
  const [draft, setDraft] = useState<Draft | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [status, setStatus] = useState('');
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let cancelled = false;
    notificationsApi
      .getQuietHours()
      .then((q) => {
        if (cancelled) return;
        const { configured, ...rest } = q;
        // Never saved: suggest the browser's zone instead of the server default.
        const initial = configured ? rest : { ...rest, timezone: browserTimezone() };
        setSaved(configured ? rest : null);
        setDraft(initial);
      })
      .catch((err) => {
        if (!cancelled) setLoadError(errorDetail(err, 'Could not load quiet hours'));
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const zones = useMemo(() => timezoneNames(draft ? [draft.timezone] : []), [draft]);

  if (loadError) {
    return (
      <p role="alert" className="text-sm text-red-600 dark:text-red-400">
        {loadError}
      </p>
    );
  }
  if (!draft) {
    return (
      <div className="flex items-center gap-2 text-gray-500">
        <Loader2 size={16} className="animate-spin" aria-hidden="true" />
        Loading...
      </div>
    );
  }

  const update = (patch: Partial<Draft>) => {
    setDraft({ ...draft, ...patch });
    setStatus('');
    setError(null);
  };
  const sameTimes = draft.from_minute === draft.to_minute;
  const dirty = saved === null || JSON.stringify(saved) !== JSON.stringify(draft);

  const save = async () => {
    if (sameTimes) return;
    setBusy(true);
    setError(null);
    try {
      const { configured: _configured, ...stored } = await notificationsApi.updateQuietHours(draft);
      setSaved(stored);
      setDraft(stored);
      setStatus('Quiet hours saved');
    } catch (err) {
      setError(errorDetail(err, 'Could not save quiet hours'));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="grid gap-3" data-testid="quiet-hours">
      <div className="flex flex-wrap items-center gap-x-4 gap-y-2 text-sm text-gray-700 dark:text-gray-300">
        <span className="inline-flex items-center gap-2 font-semibold">
          <Switch checked={draft.enabled} label="Quiet hours" onChange={() => update({ enabled: !draft.enabled })} />
          Quiet hours
        </span>
        <label className="inline-flex items-center gap-2">
          From
          <input
            type="time"
            className={timeInput}
            value={minutesToTime(draft.from_minute)}
            onChange={(e) => {
              const m = timeToMinutes(e.target.value);
              if (m !== null) update({ from_minute: m });
            }}
          />
        </label>
        <label className="inline-flex items-center gap-2">
          to
          <input
            type="time"
            className={timeInput}
            value={minutesToTime(draft.to_minute)}
            onChange={(e) => {
              const m = timeToMinutes(e.target.value);
              if (m !== null) update({ to_minute: m });
            }}
          />
        </label>
        <label className="inline-flex items-center gap-2">
          <span className="sr-only">Time zone</span>
          <select
            aria-label="Time zone"
            className={`${timeInput} max-w-[16rem]`}
            value={draft.timezone}
            onChange={(e) => update({ timezone: e.target.value })}
          >
            {zones.map((z) => (
              <option key={z} value={z}>
                {z}
              </option>
            ))}
          </select>
        </label>
      </div>
      <span className="inline-flex items-center gap-2 text-sm font-semibold text-gray-700 dark:text-gray-300">
        <Switch
          checked={draft.critical_bypass}
          label="Critical notifications always come through"
          onChange={() => update({ critical_bypass: !draft.critical_bypass })}
        />
        Critical notifications always come through
      </span>
      <p className="max-w-[72ch] text-[0.82rem] text-gray-500 dark:text-gray-400" data-testid="quiet-hours-preview">
        {quietHoursPreview(draft)}
      </p>
      {sameTimes && <p className="text-sm text-amber-700 dark:text-amber-300">Pick different start and end times.</p>}
      <div className="flex items-center gap-3">
        <button
          type="button"
          onClick={save}
          disabled={busy || sameTimes || !dirty}
          className={`rounded-lg border border-blue-600 bg-blue-600 px-3 py-1.5 text-[0.82rem] font-semibold text-white hover:bg-blue-700 disabled:opacity-50 disabled:cursor-not-allowed ${focusRing}`}
        >
          {busy ? 'Saving…' : 'Save quiet hours'}
        </button>
        <p role="status" aria-live="polite" className="text-sm text-emerald-700 dark:text-emerald-300">
          {status}
        </p>
      </div>
      {error && (
        <p role="alert" className="text-sm text-red-600 dark:text-red-400">
          {error}
        </p>
      )}
    </div>
  );
}

/** Account page card. */
export default function QuietHoursSection() {
  return (
    <div className="bg-white dark:bg-gray-800 rounded-lg border border-gray-200 dark:border-gray-700 p-6">
      <h3 className="text-lg font-semibold text-gray-900 dark:text-white mb-1 flex items-center gap-2">
        <Moon size={20} className="text-blue-500" aria-hidden="true" />
        Quiet hours
      </h3>
      <p className="text-sm text-gray-500 dark:text-gray-400 mb-4">{QUIET_HOURS_DESCRIPTION}</p>
      <QuietHoursForm />
    </div>
  );
}
