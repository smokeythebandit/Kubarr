import { useEffect, useMemo, useState } from 'react';
import type { FormEvent } from 'react';
import { useAuth } from '../../../contexts/AuthContext';
import { updatesApi } from '../../../api/updates';
import type { UpdateSettingsResponse } from '../../../api/updates';
import { SettingsCard, SettingsTabLayout } from './SettingsTabLayout';
import { WeekScheduleGrid } from '../WeekScheduleGrid';
import { countHours, normalizeSchedule, presetSchedule } from '../weekSchedule';
import type { WeekSchedule } from '../weekSchedule';

const inputClass = 'w-full px-3 py-2 border border-gray-200 dark:border-gray-600 rounded-lg bg-white dark:bg-gray-700 text-gray-900 dark:text-white focus:ring-2 focus:ring-blue-500 focus:border-transparent disabled:opacity-60';

type ApiErr = { response?: { data?: { detail?: string } }; message?: string };

function browserTimezone(): string | null {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone || null;
  } catch {
    return null;
  }
}

function timezoneNames(): string[] {
  try {
    const supported = (Intl as unknown as { supportedValuesOf?: (key: string) => string[] }).supportedValuesOf;
    const names = supported ? supported('timeZone') : [];
    return names.includes('UTC') ? names : ['UTC', ...names];
  } catch {
    return ['UTC'];
  }
}

function formatNextWindow(iso: string, timezone: string): string {
  const date = new Date(iso);
  try {
    return date.toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short', timeZone: timezone });
  } catch {
    return date.toLocaleString();
  }
}

export function UpdatesTab() {
  const canManage = useAuth().hasPermission('settings.manage');
  const [saved, setSaved] = useState<UpdateSettingsResponse | null>(null);
  const [enabled, setEnabled] = useState(true);
  const [schedule, setSchedule] = useState<WeekSchedule>(() => presetSchedule('nightly'));
  const [timezone, setTimezone] = useState('UTC');
  const [healthTimeout, setHealthTimeout] = useState('10');
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [justSaved, setJustSaved] = useState(false);

  const apply = (s: UpdateSettingsResponse, preselectBrowserZone: boolean) => {
    setSaved(s);
    setEnabled(s.enabled);
    setSchedule(normalizeSchedule(s.schedule));
    setHealthTimeout(String(s.health_timeout_minutes));
    // Until an owner saves a zone the server's UTC is only a default:
    // start from the zone this browser is in.
    setTimezone(preselectBrowserZone && !s.timezone_saved ? browserTimezone() ?? 'UTC' : s.timezone);
  };

  useEffect(() => {
    let cancelled = false;
    updatesApi.getSettings()
      .then((s) => { if (!cancelled) apply(s, true); })
      .catch((err: ApiErr) => { if (!cancelled) setLoadError(err.response?.data?.detail || err.message || 'Failed to load update settings'); })
      .finally(() => { if (!cancelled) setLoading(false); });
    return () => { cancelled = true; };
  }, []);

  const zones = useMemo(() => {
    const names = timezoneNames();
    return names.includes(timezone) ? names : [timezone, ...names];
  }, [timezone]);

  const handleSubmit = async (event: FormEvent) => {
    event.preventDefault();
    setSaving(true);
    setSaveError(null);
    setJustSaved(false);
    try {
      const result = await updatesApi.saveSettings({
        enabled,
        schedule,
        timezone,
        health_timeout_minutes: Number(healthTimeout),
      });
      apply(result, false);
      setJustSaved(true);
    } catch (err) {
      const e = err as ApiErr;
      setSaveError(e.response?.data?.detail || e.message || 'Failed to save update settings');
    } finally {
      setSaving(false);
    }
  };

  const disabled = !canManage || saving;

  return (
    <SettingsTabLayout
      title="Updates"
      description="Apps are updated automatically during the hours you choose, one app at a time. Patch and minor chart updates are applied; major updates always wait for you. If an app is unhealthy after an update, Kubarr rolls it back."
    >
      {loadError && <div className="p-3 bg-red-50 dark:bg-red-900/20 text-red-700 dark:text-red-300 rounded-lg text-sm">{loadError}</div>}
      {loading ? (
        <div className="py-8 text-center text-gray-500">Loading update settings...</div>
      ) : saved && (
        <SettingsCard>
          <form onSubmit={handleSubmit} className="space-y-6">
            <div className="flex items-center justify-between">
              <div>
                <div className="font-medium text-gray-900 dark:text-white">Automatic updates</div>
                <div className="text-sm text-gray-500 dark:text-gray-400 mt-1">
                  {saved.enabled && saved.next_window_start
                    ? `Next window: ${formatNextWindow(saved.next_window_start, saved.timezone)}`
                    : saved.enabled ? 'No update hours selected' : 'Automatic updates are off'}
                </div>
              </div>
              <button
                type="button"
                role="switch"
                aria-label="Automatic updates"
                aria-checked={enabled}
                disabled={disabled}
                onClick={() => setEnabled(!enabled)}
                className={`relative inline-flex h-6 w-11 items-center rounded-full transition-colors focus:outline-none focus:ring-2 focus:ring-blue-500 ${enabled ? 'bg-blue-600' : 'bg-gray-300 dark:bg-gray-600'} ${disabled ? 'opacity-50 cursor-not-allowed' : ''}`}
              >
                <span className={`inline-block h-4 w-4 transform rounded-full bg-white transition-transform ${enabled ? 'translate-x-6' : 'translate-x-1'}`} />
              </button>
            </div>

            <WeekScheduleGrid
              schedule={schedule}
              onChange={setSchedule}
              timezone={timezone}
              active={enabled}
              readOnly={disabled}
            />
            {enabled && countHours(schedule) === 0 && (
              <div className="p-3 bg-amber-50 dark:bg-amber-900/20 text-amber-800 dark:text-amber-200 rounded-lg text-sm">
                No hours selected — automatic updates won&apos;t run.
              </div>
            )}
            <p className="text-xs text-gray-500 dark:text-gray-400">
              An update that starts inside a window is always allowed to finish, including its health check and a rollback if needed, even when that runs past the end of the window. Major updates never run automatically.
            </p>

            <div className="grid gap-4 sm:grid-cols-2">
              <div>
                <label htmlFor="update-timezone" className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">Timezone</label>
                <select id="update-timezone" value={timezone} onChange={(e) => setTimezone(e.target.value)} disabled={disabled} className={inputClass}>
                  {zones.map((name) => <option key={name} value={name}>{name}</option>)}
                </select>
                {!saved.timezone_saved && (
                  <p className="mt-1 text-xs text-amber-700 dark:text-amber-300">Not saved yet — Kubarr uses UTC until you save.</p>
                )}
              </div>
              <div>
                <label htmlFor="update-health-timeout" className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">Health check timeout (minutes)</label>
                <input id="update-health-timeout" type="number" required min={5} max={60} value={healthTimeout} onChange={(e) => setHealthTimeout(e.target.value)} disabled={disabled} className={inputClass} />
              </div>
            </div>

            {saveError && <div role="alert" className="p-3 bg-red-50 dark:bg-red-900/20 text-red-700 dark:text-red-300 rounded-lg text-sm">{saveError}</div>}
            {justSaved && <div role="status" className="p-3 bg-green-50 dark:bg-green-900/20 text-green-700 dark:text-green-300 rounded-lg text-sm">Update settings saved.</div>}

            {canManage && (
              <button type="submit" disabled={saving} className="px-4 py-2 bg-blue-600 hover:bg-blue-700 disabled:bg-blue-400 text-white rounded-lg font-medium transition-colors">
                {saving ? 'Saving...' : 'Save'}
              </button>
            )}
          </form>
        </SettingsCard>
      )}
    </SettingsTabLayout>
  );
}
