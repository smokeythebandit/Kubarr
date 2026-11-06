import { useEffect, useMemo, useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { appsApi } from '../../../api/apps';
import type { AutoUpdateConfig } from '../../../types/app';
import { SettingsCard, SettingsTabLayout } from './SettingsTabLayout';

const DAYS = [
  { id: 'mon', label: 'Mon' },
  { id: 'tue', label: 'Tue' },
  { id: 'wed', label: 'Wed' },
  { id: 'thu', label: 'Thu' },
  { id: 'fri', label: 'Fri' },
  { id: 'sat', label: 'Sat' },
  { id: 'sun', label: 'Sun' },
];

function browserTimezone(): string {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone || 'UTC';
  } catch {
    return 'UTC';
  }
}

function timezones(current: string): string[] {
  let zones: string[] = [];
  try {
    zones = (Intl as unknown as { supportedValuesOf?: (key: string) => string[] }).supportedValuesOf?.('timeZone') ?? [];
  } catch {
    zones = [];
  }
  // Always offer UTC, the browser's zone and whatever is saved, even on a
  // browser that cannot list zones.
  return Array.from(new Set(['UTC', browserTimezone(), current, ...zones])).sort();
}

function Switch({ label, description, checked, disabled, onChange }: {
  label: string;
  description: string;
  checked: boolean;
  disabled?: boolean;
  onChange: (value: boolean) => void;
}) {
  return (
    <div className="flex items-center justify-between gap-4 py-4">
      <div className="flex-1">
        <div className="font-medium text-gray-900 dark:text-white">{label}</div>
        <div className="mt-1 text-sm text-gray-500 dark:text-gray-400">{description}</div>
      </div>
      <button
        type="button"
        role="switch"
        aria-label={label}
        aria-checked={checked}
        disabled={disabled}
        onClick={() => onChange(!checked)}
        className={`relative inline-flex h-6 w-11 flex-shrink-0 items-center rounded-full transition-colors focus:outline-none focus:ring-2 focus:ring-blue-500 focus:ring-offset-2 focus:ring-offset-white dark:focus:ring-offset-gray-800 ${
          checked ? 'bg-blue-600' : 'bg-gray-300 dark:bg-gray-600'
        } ${disabled ? 'cursor-not-allowed opacity-50' : ''}`}
      >
        <span className={`inline-block h-4 w-4 transform rounded-full bg-white transition-transform ${checked ? 'translate-x-6' : 'translate-x-1'}`} />
      </button>
    </div>
  );
}

/** Explains a window in words, so "22:00–02:00" reads as overnight. */
function describeWindow(config: AutoUpdateConfig): string {
  const { days, start, end, timezone } = config.window;
  const dayText = days.length === 7
    ? 'Every day'
    : DAYS.filter(day => days.includes(day.id)).map(day => day.label).join(', ');
  const overnight = end <= start ? ' (overnight)' : '';
  return `${dayText}, ${start}–${end}${overnight}, ${timezone}`;
}

export function UpdatesTab() {
  const queryClient = useQueryClient();
  const { data: status, isLoading, isError } = useQuery({
    queryKey: ['auto-update'],
    queryFn: appsApi.getAutoUpdate,
  });
  const [draft, setDraft] = useState<AutoUpdateConfig | null>(null);
  const [message, setMessage] = useState<{ text: string; ok: boolean } | null>(null);

  // Start editing from what the server holds; a fresh install has never saved
  // a timezone, so suggest the browser's instead of UTC.
  useEffect(() => {
    if (status && !draft) {
      const saved = status.config;
      const neverConfigured = !saved.enabled && saved.window.timezone === 'UTC';
      setDraft(neverConfigured ? { ...saved, window: { ...saved.window, timezone: browserTimezone() } } : saved);
    }
  }, [status, draft]);

  const zoneOptions = useMemo(() => timezones(draft?.window.timezone ?? 'UTC'), [draft?.window.timezone]);

  const save = useMutation({
    mutationFn: appsApi.setAutoUpdate,
    onSuccess: result => {
      queryClient.setQueryData(['auto-update'], result);
      setDraft(result.config);
      setMessage({ text: 'Saved', ok: true });
    },
    onError: (error: { response?: { data?: { detail?: string } }; message: string }) => {
      setMessage({ text: error.response?.data?.detail || error.message, ok: false });
    },
  });

  if (isLoading || (!draft && !isError)) {
    return (
      <SettingsTabLayout title="Updates" description="Keep apps up to date automatically.">
        <div className="flex justify-center py-12">
          <div className="h-8 w-8 animate-spin rounded-full border-b-2 border-blue-500" />
        </div>
      </SettingsTabLayout>
    );
  }

  if (isError || !draft) {
    return (
      <SettingsTabLayout title="Updates" description="Keep apps up to date automatically.">
        <SettingsCard>
          <p className="text-sm text-red-600 dark:text-red-400">Could not load the update settings.</p>
        </SettingsCard>
      </SettingsTabLayout>
    );
  }

  const setWindow = (patch: Partial<AutoUpdateConfig['window']>) =>
    setDraft({ ...draft, window: { ...draft.window, ...patch } });
  const toggleDay = (day: string) =>
    setWindow({
      days: draft.window.days.includes(day)
        ? draft.window.days.filter(d => d !== day)
        : [...draft.window.days, day],
    });

  const dirty = JSON.stringify(draft) !== JSON.stringify(status?.config);
  const invalid = draft.window.days.length === 0 || draft.window.start === draft.window.end;
  const inputClass = 'rounded-lg border border-gray-200 bg-white px-3 py-2 text-sm text-gray-900 focus:outline-none focus:ring-2 focus:ring-blue-500 dark:border-gray-700 dark:bg-gray-900 dark:text-white';

  return (
    <SettingsTabLayout
      title="Updates"
      description="Update apps automatically during a window you choose, so security fixes land without anyone pressing a button."
    >
      <SettingsCard>
        <div className="divide-y divide-gray-200 dark:divide-gray-700">
          <Switch
            label="Automatic updates"
            description="Update installed apps one at a time, only inside the window below. Unhealthy apps are skipped, and an update that fails is not retried until a newer version appears."
            checked={draft.enabled}
            onChange={enabled => setDraft({ ...draft, enabled })}
          />
          <Switch
            label="Include major versions"
            description="Also take jumps like 1.x to 2.x. These are the updates most likely to need attention, so they are off by default. Security fixes usually ship as minor or patch releases."
            checked={draft.allow_major}
            disabled={!draft.enabled}
            onChange={allow_major => setDraft({ ...draft, allow_major })}
          />
        </div>
      </SettingsCard>

      <SettingsCard className={draft.enabled ? '' : 'opacity-60'}>
        <h4 className="font-medium text-gray-900 dark:text-white">Update window</h4>
        <p className="mt-1 text-sm text-gray-500 dark:text-gray-400">
          Automatic updates only start inside this window. An update that starts near the end may finish after it closes.
        </p>

        <fieldset className="mt-4" disabled={!draft.enabled}>
          <legend className="text-sm font-medium text-gray-700 dark:text-gray-300">Days</legend>
          <div className="mt-2 flex flex-wrap gap-2">
            {DAYS.map(day => {
              const on = draft.window.days.includes(day.id);
              return (
                <button
                  key={day.id}
                  type="button"
                  aria-pressed={on}
                  onClick={() => toggleDay(day.id)}
                  className={`rounded-lg border px-3 py-1.5 text-sm font-medium transition-colors ${
                    on
                      ? 'border-blue-600 bg-blue-600 text-white'
                      : 'border-gray-200 bg-white text-gray-700 hover:bg-gray-50 dark:border-gray-700 dark:bg-gray-900 dark:text-gray-300 dark:hover:bg-gray-800'
                  }`}
                >
                  {day.label}
                </button>
              );
            })}
          </div>
        </fieldset>

        <fieldset className="mt-4 grid gap-4 sm:grid-cols-3" disabled={!draft.enabled}>
          <label className="flex flex-col gap-1 text-sm font-medium text-gray-700 dark:text-gray-300">
            From
            <input type="time" value={draft.window.start} onChange={e => setWindow({ start: e.target.value })} className={inputClass} />
          </label>
          <label className="flex flex-col gap-1 text-sm font-medium text-gray-700 dark:text-gray-300">
            Until
            <input type="time" value={draft.window.end} onChange={e => setWindow({ end: e.target.value })} className={inputClass} />
          </label>
          <label className="flex flex-col gap-1 text-sm font-medium text-gray-700 dark:text-gray-300">
            Timezone
            <select value={draft.window.timezone} onChange={e => setWindow({ timezone: e.target.value })} className={inputClass}>
              {zoneOptions.map(zone => <option key={zone} value={zone}>{zone}</option>)}
            </select>
          </label>
        </fieldset>

        <p className="mt-4 text-sm text-gray-600 dark:text-gray-400">
          {draft.window.days.length === 0
            ? 'Pick at least one day.'
            : draft.window.start === draft.window.end
              ? 'The window needs a different start and end time.'
              : describeWindow(draft)}
        </p>
        {status && !dirty && status.config.enabled && (
          <p className="mt-1 text-sm text-gray-500 dark:text-gray-400">
            {status.in_window
              ? 'The window is open now.'
              : status.next_window_start
                ? `Next window opens ${new Date(status.next_window_start).toLocaleString()}.`
                : null}
          </p>
        )}
      </SettingsCard>

      <div className="flex items-center justify-end gap-3">
        {message && (
          <span className={`text-sm ${message.ok ? 'text-green-600 dark:text-green-400' : 'text-red-600 dark:text-red-400'}`}>
            {message.text}
          </span>
        )}
        <button
          type="button"
          disabled={!dirty || invalid || save.isPending}
          onClick={() => { setMessage(null); save.mutate(draft); }}
          className="rounded-lg bg-blue-600 px-4 py-2 text-sm font-medium text-white transition-colors hover:bg-blue-700 disabled:cursor-not-allowed disabled:opacity-50"
        >
          {save.isPending ? 'Saving…' : 'Save'}
        </button>
      </div>
    </SettingsTabLayout>
  );
}
