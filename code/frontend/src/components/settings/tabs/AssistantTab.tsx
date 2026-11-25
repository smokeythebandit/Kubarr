import { useCallback, useEffect, useState } from 'react';
import type { FormEvent } from 'react';
import { useAuth } from '../../../contexts/AuthContext';
import { ASSISTANT_MODELS, assistantApi } from '../../../api/assistant';
import type { AssistantEffort, AssistantModel, AssistantSettings, AssistantStatus, KeyTestResult } from '../../../api/assistant';
import { SettingsCard, SettingsTabLayout } from './SettingsTabLayout';

const inputClass = 'w-full px-3 py-2 border border-gray-200 dark:border-gray-600 rounded-lg bg-white dark:bg-gray-700 text-gray-900 dark:text-white focus:ring-2 focus:ring-blue-500 focus:border-transparent disabled:opacity-60';
const labelClass = 'block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1';
const secondaryButton = 'px-4 py-2 border border-gray-200 dark:border-gray-600 text-gray-700 dark:text-gray-200 hover:bg-gray-50 dark:hover:bg-gray-700 disabled:opacity-50 rounded-lg font-medium transition-colors';

const EFFORTS: { id: AssistantEffort; label: string }[] = [
  { id: 'low', label: 'Low (fastest, cheapest)' },
  { id: 'medium', label: 'Medium' },
  { id: 'high', label: 'High (most thorough)' },
];

type ApiErr = { response?: { data?: { detail?: string } }; message?: string };

function errorText(err: unknown, fallback: string): string {
  const e = err as ApiErr;
  return e.response?.data?.detail || e.message || fallback;
}

function usd(value: number): string {
  return `$${value.toFixed(2)}`;
}

/** This month's spend against the budget, as a bar. */
function SpendBar({ status }: { status: AssistantStatus }) {
  const budget = status.monthly_budget_usd;
  const ratio = budget > 0 ? Math.min(status.month_spend_usd / budget, 1) : 1;
  const percent = Math.round(ratio * 100);
  const color = status.paused ? 'bg-red-500' : ratio >= 0.8 ? 'bg-amber-500' : 'bg-blue-600';
  return (
    <div>
      <div className="flex items-baseline justify-between text-sm">
        <span className="font-medium text-gray-900 dark:text-white">This month</span>
        <span className="text-gray-600 dark:text-gray-300">
          {usd(status.month_spend_usd)} of {usd(budget)}
        </span>
      </div>
      <div
        role="progressbar"
        aria-label="Spend this month"
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={percent}
        className="mt-2 h-2 w-full rounded-full bg-gray-200 dark:bg-gray-700 overflow-hidden"
      >
        <div className={`h-full ${color} transition-all`} style={{ width: `${percent}%` }} />
      </div>
      {status.paused && (
        <p className="mt-2 text-sm text-red-700 dark:text-red-300">
          The monthly budget is used up. The assistant is paused until next month or until the budget is raised.
        </p>
      )}
      {!status.paused && ratio >= 0.8 && (
        <p className="mt-2 text-sm text-amber-700 dark:text-amber-300">Over 80 % of this month&apos;s budget is used.</p>
      )}
    </div>
  );
}

export function AssistantTab() {
  const canManage = useAuth().hasPermission('settings.manage');
  const [status, setStatus] = useState<AssistantStatus | null>(null);
  const [saved, setSaved] = useState<AssistantSettings | null>(null);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);

  const [keyInput, setKeyInput] = useState('');
  const [keyBusy, setKeyBusy] = useState(false);
  const [keyError, setKeyError] = useState<string | null>(null);
  const [keyNotice, setKeyNotice] = useState<string | null>(null);
  const [testResult, setTestResult] = useState<KeyTestResult | null>(null);

  const [enabled, setEnabled] = useState(true);
  const [model, setModel] = useState<AssistantModel>('claude-opus-5-5');
  const [effort, setEffort] = useState<AssistantEffort>('low');
  const [budget, setBudget] = useState('20');
  const [retention, setRetention] = useState('30');
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [justSaved, setJustSaved] = useState(false);

  const applySettings = (s: AssistantSettings) => {
    setSaved(s);
    setEnabled(s.enabled);
    setModel(s.model);
    setEffort(s.effort);
    setBudget(String(s.monthly_budget_usd));
    setRetention(String(s.retention_days));
  };

  const refreshStatus = useCallback(async () => {
    try {
      setStatus(await assistantApi.getStatus());
    } catch (err) {
      setLoadError(errorText(err, 'Failed to load assistant status'));
    }
  }, []);

  useEffect(() => {
    let cancelled = false;
    Promise.all([assistantApi.getStatus(), assistantApi.getSettings()])
      .then(([st, s]) => {
        if (cancelled) return;
        setStatus(st);
        applySettings(s);
      })
      .catch((err) => { if (!cancelled) setLoadError(errorText(err, 'Failed to load assistant settings')); })
      .finally(() => { if (!cancelled) setLoading(false); });
    return () => { cancelled = true; };
  }, []);

  const resetKeyMessages = () => {
    setKeyError(null);
    setKeyNotice(null);
    setTestResult(null);
  };

  const handleSaveKey = async (event: FormEvent) => {
    event.preventDefault();
    if (!keyInput.trim()) return;
    resetKeyMessages();
    setKeyBusy(true);
    try {
      await assistantApi.setKey(keyInput.trim());
      setKeyInput('');
      setKeyNotice('API key saved.');
      await refreshStatus();
    } catch (err) {
      setKeyError(errorText(err, 'Failed to save the API key'));
    } finally {
      setKeyBusy(false);
    }
  };

  const handleRemoveKey = async () => {
    if (!window.confirm('Remove the Anthropic API key? The assistant stops working until a new key is set.')) return;
    resetKeyMessages();
    setKeyBusy(true);
    try {
      await assistantApi.removeKey();
      setKeyNotice('API key removed.');
      await refreshStatus();
    } catch (err) {
      setKeyError(errorText(err, 'Failed to remove the API key'));
    } finally {
      setKeyBusy(false);
    }
  };

  const handleTestKey = async () => {
    resetKeyMessages();
    setKeyBusy(true);
    try {
      setTestResult(await assistantApi.testKey());
      await refreshStatus();
    } catch (err) {
      setTestResult({ ok: false, error: errorText(err, 'Failed to test the API key') });
    } finally {
      setKeyBusy(false);
    }
  };

  const handleSubmit = async (event: FormEvent) => {
    event.preventDefault();
    setSaving(true);
    setSaveError(null);
    setJustSaved(false);
    try {
      const result = await assistantApi.saveSettings({
        enabled,
        model,
        effort,
        monthly_budget_usd: Number(budget),
        retention_days: Number(retention),
      });
      applySettings(result);
      setJustSaved(true);
      await refreshStatus();
    } catch (err) {
      setSaveError(errorText(err, 'Failed to save assistant settings'));
    } finally {
      setSaving(false);
    }
  };

  const disabled = !canManage || saving;
  const configured = status?.configured ?? false;

  return (
    <SettingsTabLayout
      title="Assistant"
      description="An AI assistant that answers questions about your apps and cluster, using your own Anthropic API key. Usage is billed to your Anthropic account; the monthly budget pauses the assistant when it is reached."
    >
      {loadError && <div className="p-3 bg-red-50 dark:bg-red-900/20 text-red-700 dark:text-red-300 rounded-lg text-sm">{loadError}</div>}
      {loading ? (
        <div className="py-8 text-center text-gray-500">Loading assistant settings...</div>
      ) : status && saved && (
        <>
          <SettingsCard>
            <div className="space-y-4">
              <div className="flex items-center justify-between gap-4">
                <div>
                  <div className="font-medium text-gray-900 dark:text-white">Anthropic API key</div>
                  <div className="text-sm text-gray-500 dark:text-gray-400 mt-1">
                    Stored in a Kubernetes Secret in Kubarr&apos;s namespace. It is never shown again after saving.
                  </div>
                </div>
                <span
                  data-testid="key-status"
                  className={`shrink-0 px-2.5 py-1 rounded-full text-xs font-medium ${configured ? 'bg-green-100 text-green-800 dark:bg-green-900/40 dark:text-green-300' : 'bg-gray-100 text-gray-700 dark:bg-gray-700 dark:text-gray-300'}`}
                >
                  {configured ? 'Configured' : 'Not set'}
                </span>
              </div>

              {canManage && (
                <form onSubmit={handleSaveKey} className="flex flex-col gap-2 sm:flex-row">
                  <label htmlFor="assistant-api-key" className="sr-only">API key</label>
                  <input
                    id="assistant-api-key"
                    type="password"
                    autoComplete="off"
                    spellCheck={false}
                    placeholder={configured ? 'Paste a new key to replace the current one' : 'sk-ant-...'}
                    value={keyInput}
                    onChange={(e) => setKeyInput(e.target.value)}
                    disabled={keyBusy}
                    className={inputClass}
                  />
                  <button type="submit" disabled={keyBusy || !keyInput.trim()} className="shrink-0 px-4 py-2 bg-blue-600 hover:bg-blue-700 disabled:bg-blue-400 text-white rounded-lg font-medium transition-colors">
                    {configured ? 'Replace key' : 'Save key'}
                  </button>
                </form>
              )}

              {canManage && configured && (
                <div className="flex flex-wrap gap-2">
                  <button type="button" onClick={handleTestKey} disabled={keyBusy} className={secondaryButton}>Test key</button>
                  <button type="button" onClick={handleRemoveKey} disabled={keyBusy} className="px-4 py-2 border border-red-200 dark:border-red-800 text-red-700 dark:text-red-300 hover:bg-red-50 dark:hover:bg-red-900/20 disabled:opacity-50 rounded-lg font-medium transition-colors">
                    Remove key
                  </button>
                </div>
              )}

              {keyError && <div role="alert" className="p-3 bg-red-50 dark:bg-red-900/20 text-red-700 dark:text-red-300 rounded-lg text-sm">{keyError}</div>}
              {keyNotice && <div role="status" className="p-3 bg-green-50 dark:bg-green-900/20 text-green-700 dark:text-green-300 rounded-lg text-sm">{keyNotice}</div>}
              {testResult && (testResult.ok ? (
                <div role="status" className="p-3 bg-green-50 dark:bg-green-900/20 text-green-700 dark:text-green-300 rounded-lg text-sm">The key works.</div>
              ) : (
                <div role="alert" className="p-3 bg-red-50 dark:bg-red-900/20 text-red-700 dark:text-red-300 rounded-lg text-sm">Key test failed: {testResult.error ?? 'unknown error'}</div>
              ))}
            </div>
          </SettingsCard>

          <SettingsCard>
            <SpendBar status={status} />
          </SettingsCard>

          <SettingsCard>
            <form onSubmit={handleSubmit} className="space-y-6">
              <div className="flex items-center justify-between">
                <div>
                  <div className="font-medium text-gray-900 dark:text-white">Assistant</div>
                  <div className="text-sm text-gray-500 dark:text-gray-400 mt-1">
                    {!configured ? 'Set an API key to use the assistant.' : enabled ? 'Available to roles with the "Chat with the AI assistant" permission.' : 'Switched off for everyone.'}
                  </div>
                </div>
                <button
                  type="button"
                  role="switch"
                  aria-label="Enable assistant"
                  aria-checked={enabled}
                  disabled={disabled}
                  onClick={() => setEnabled(!enabled)}
                  className={`relative inline-flex h-6 w-11 items-center rounded-full transition-colors focus:outline-none focus:ring-2 focus:ring-blue-500 ${enabled ? 'bg-blue-600' : 'bg-gray-300 dark:bg-gray-600'} ${disabled ? 'opacity-50 cursor-not-allowed' : ''}`}
                >
                  <span className={`inline-block h-4 w-4 transform rounded-full bg-white transition-transform ${enabled ? 'translate-x-6' : 'translate-x-1'}`} />
                </button>
              </div>

              <div className="grid gap-4 sm:grid-cols-2">
                <div>
                  <label htmlFor="assistant-model" className={labelClass}>Model</label>
                  <select id="assistant-model" value={model} onChange={(e) => setModel(e.target.value as AssistantModel)} disabled={disabled} className={inputClass}>
                    {ASSISTANT_MODELS.map((m) => <option key={m.id} value={m.id}>{m.label} — {m.hint}</option>)}
                  </select>
                </div>
                <div>
                  <label htmlFor="assistant-effort" className={labelClass}>Effort</label>
                  <select id="assistant-effort" value={effort} onChange={(e) => setEffort(e.target.value as AssistantEffort)} disabled={disabled} className={inputClass}>
                    {EFFORTS.map((e) => <option key={e.id} value={e.id}>{e.label}</option>)}
                  </select>
                </div>
                <div>
                  <label htmlFor="assistant-budget" className={labelClass}>Monthly budget (USD)</label>
                  <input id="assistant-budget" type="number" required min={0} max={100000} step="any" value={budget} onChange={(e) => setBudget(e.target.value)} disabled={disabled} className={inputClass} />
                  <p className="mt-1 text-xs text-gray-500 dark:text-gray-400">Admins are notified at 80 %; the assistant pauses at 100 %.</p>
                </div>
                <div>
                  <label htmlFor="assistant-retention" className={labelClass}>Keep conversations (days)</label>
                  <input id="assistant-retention" type="number" required min={1} max={365} value={retention} onChange={(e) => setRetention(e.target.value)} disabled={disabled} className={inputClass} />
                </div>
              </div>

              {saveError && <div role="alert" className="p-3 bg-red-50 dark:bg-red-900/20 text-red-700 dark:text-red-300 rounded-lg text-sm">{saveError}</div>}
              {justSaved && <div role="status" className="p-3 bg-green-50 dark:bg-green-900/20 text-green-700 dark:text-green-300 rounded-lg text-sm">Assistant settings saved.</div>}

              {canManage && (
                <button type="submit" disabled={saving} className="px-4 py-2 bg-blue-600 hover:bg-blue-700 disabled:bg-blue-400 text-white rounded-lg font-medium transition-colors">
                  {saving ? 'Saving...' : 'Save'}
                </button>
              )}
            </form>
          </SettingsCard>
        </>
      )}
    </SettingsTabLayout>
  );
}
