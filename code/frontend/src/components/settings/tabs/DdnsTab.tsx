import { useEffect, useMemo, useState } from 'react';
import type { FormEvent } from 'react';
import { Globe2, Pencil, Plus, Trash2, X } from 'lucide-react';
import { defaultDnsCapabilities, domainsApi, type DynamicDnsCapabilities, type DynamicDnsProfile, type DynamicDnsProfileRequest } from '../../../api/domains';
import { SettingsCard, SettingsTabLayout } from './SettingsTabLayout';

const inputClass = 'w-full px-3 py-2 border border-gray-200 dark:border-gray-600 rounded-lg bg-white dark:bg-gray-700 text-gray-900 dark:text-white focus:ring-2 focus:ring-blue-500 focus:border-transparent';

export function DdnsTab() {
  const [profiles, setProfiles] = useState<DynamicDnsProfile[]>([]);
  const [editing, setEditing] = useState<DynamicDnsProfile | null>(null);
  const [showForm, setShowForm] = useState(false);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const loadProfiles = async () => {
    try {
      setLoading(true);
      setProfiles(await domainsApi.listDdnsProfiles());
      setError(null);
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to load Dynamic DNS profiles');
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => { loadProfiles(); }, []);

  const handleDelete = async (profile: DynamicDnsProfile) => {
    if (!confirm(`Delete Dynamic DNS profile "${profile.name}"?`)) return;
    await domainsApi.deleteDdnsProfile(profile.id);
    await loadProfiles();
  };

  return (
    <SettingsTabLayout
      title="Dynamic DNS"
      description="Manage DNS provider profiles used for A/AAAA updates, wildcard DNS, and DNS-01 TXT challenges."
      actions={<button onClick={() => { setEditing(null); setShowForm(true); }} className="inline-flex items-center gap-2 px-4 py-2 bg-blue-600 hover:bg-blue-700 text-white rounded-lg transition-colors"><Plus size={18} /> Add Profile</button>}
    >
      {error && <div className="p-3 bg-red-50 dark:bg-red-900/20 text-red-700 dark:text-red-300 rounded-lg text-sm">{error}</div>}
      {loading ? <div className="py-8 text-center text-gray-500">Loading profiles...</div> : profiles.length === 0 ? (
        <SettingsCard><div className="py-8 text-center text-gray-500 dark:text-gray-400">No Dynamic DNS profiles configured.</div></SettingsCard>
      ) : (
        <div className="grid gap-4">
          {profiles.map((profile) => (
            <SettingsCard key={profile.id}>
              <div className="flex items-start justify-between gap-4">
                <div className="flex items-start gap-3">
                  <div className="w-10 h-10 rounded-lg bg-blue-100 dark:bg-blue-900/30 flex items-center justify-center"><Globe2 className="text-blue-600 dark:text-blue-300" size={22} /></div>
                  <div>
                    <div className="font-medium text-gray-900 dark:text-white">{profile.name}</div>
                    <div className="mt-1 flex flex-wrap gap-2 text-xs">
                      <span className="px-2 py-0.5 rounded bg-gray-100 dark:bg-gray-700 text-gray-600 dark:text-gray-300">{profile.provider}</span>
                      {profile.capabilities.txt_records && <span className="px-2 py-0.5 rounded bg-green-100 dark:bg-green-900/30 text-green-700 dark:text-green-300">TXT challenges</span>}
                      {profile.capabilities.wildcard_records && <span className="px-2 py-0.5 rounded bg-purple-100 dark:bg-purple-900/30 text-purple-700 dark:text-purple-300">Wildcard DNS</span>}
                      <span className={profile.enabled ? 'text-green-600 dark:text-green-400' : 'text-gray-500'}>{profile.enabled ? 'Enabled' : 'Disabled'}</span>
                    </div>
                  </div>
                </div>
                <div className="flex gap-2">
                  <button onClick={() => { setEditing(profile); setShowForm(true); }} className="p-2 text-gray-500 hover:text-blue-500 hover:bg-blue-50 dark:hover:bg-blue-900/20 rounded-lg"><Pencil size={18} /></button>
                  <button onClick={() => handleDelete(profile)} className="p-2 text-gray-500 hover:text-red-500 hover:bg-red-50 dark:hover:bg-red-900/20 rounded-lg"><Trash2 size={18} /></button>
                </div>
              </div>
            </SettingsCard>
          ))}
        </div>
      )}
      {showForm && <DdnsProfileModal profile={editing} onClose={() => setShowForm(false)} onSave={async () => { setShowForm(false); await loadProfiles(); }} />}
    </SettingsTabLayout>
  );
}

function DdnsProfileModal({ profile, onClose, onSave }: { profile: DynamicDnsProfile | null; onClose: () => void; onSave: () => Promise<void> }) {
  const [name, setName] = useState(profile?.name || '');
  const [provider, setProvider] = useState(profile?.provider || 'duckdns');
  const [enabled, setEnabled] = useState(profile?.enabled ?? true);
  const [capabilities, setCapabilities] = useState<DynamicDnsCapabilities>(profile?.capabilities || defaultDnsCapabilities());
  const [config, setConfig] = useState(profile?.config || { auth_type: 'token', token: '', zone: '', login: '', private_key: '', global_key: 'true', webhook_url: '' });
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const providerDefaults = useMemo(() => {
    if (provider === 'transip') {
      return { a_records: true, aaaa_records: true, cname_records: true, txt_records: true, wildcard_records: true };
    }
    if (provider === 'manual') {
      return { a_records: false, aaaa_records: false, cname_records: false, txt_records: false, wildcard_records: false };
    }
    return defaultDnsCapabilities();
  }, [provider]);

  useEffect(() => {
    if (!profile) {
      setCapabilities(providerDefaults);
    }
  }, [profile, providerDefaults]);

  const submit = async (event: FormEvent) => {
    event.preventDefault();
    const request: DynamicDnsProfileRequest = { name, provider, capabilities, config, enabled };
    try {
      setSaving(true);
      if (profile) await domainsApi.updateDdnsProfile(profile.id, request);
      else await domainsApi.createDdnsProfile(request);
      await onSave();
    } catch (err: any) {
      setError(err.response?.data?.detail || 'Failed to save Dynamic DNS profile');
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50 p-4">
      <div className="bg-white dark:bg-gray-800 rounded-lg shadow-xl w-full max-w-xl max-h-[90vh] overflow-y-auto">
        <div className="flex items-center justify-between p-4 border-b border-gray-200 dark:border-gray-700">
          <h2 className="text-lg font-semibold text-gray-900 dark:text-white">{profile ? 'Edit Dynamic DNS Profile' : 'Add Dynamic DNS Profile'}</h2>
          <button onClick={onClose} className="p-2 hover:bg-gray-100 dark:hover:bg-gray-700 rounded-lg"><X size={20} /></button>
        </div>
        <form onSubmit={submit} className="p-4 space-y-4">
          {error && <div className="p-3 bg-red-50 dark:bg-red-900/20 text-red-700 dark:text-red-300 rounded-lg text-sm">{error}</div>}
          <div><label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">Name</label><input required value={name} onChange={(e) => setName(e.target.value)} className={inputClass} placeholder="Home DNS" /></div>
          <div><label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">Provider</label><select value={provider} onChange={(e) => setProvider(e.target.value)} className={inputClass}><option value="transip">TransIP</option><option value="duckdns">DuckDNS</option><option value="noip">No-IP</option><option value="custom">Custom webhook</option><option value="manual">Manual / external DNS</option></select></div>
          {provider === 'transip' && (
            <div className="space-y-3">
              <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
                <div><label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">DNS zone</label><input value={config.zone || ''} onChange={(e) => setConfig({ ...config, zone: e.target.value })} className={inputClass} placeholder="example.com" /></div>
                <div><label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">Authentication</label><select value={config.auth_type || 'token'} onChange={(e) => setConfig({ ...config, auth_type: e.target.value })} className={inputClass}><option value="token">Access token</option><option value="private_key">Login + private key</option></select></div>
              </div>
              {(config.auth_type || 'token') === 'token' ? (
                <div><label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">Access Token</label><input type="password" value={config.token || ''} onChange={(e) => setConfig({ ...config, token: e.target.value })} className={inputClass} placeholder={profile ? 'Leave as-is or replace' : ''} /></div>
              ) : (
                <div className="space-y-3">
                  <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
                    <div><label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">Login</label><input value={config.login || ''} onChange={(e) => setConfig({ ...config, login: e.target.value })} className={inputClass} placeholder="TransIP username" /></div>
                    <label className="flex items-center gap-2 text-sm text-gray-700 dark:text-gray-300 pt-7"><input type="checkbox" checked={(config.global_key ?? 'true') === 'true'} onChange={(e) => setConfig({ ...config, global_key: String(e.target.checked) })} /> Global key</label>
                  </div>
                  <div><label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">Private Key</label><textarea value={config.private_key || ''} onChange={(e) => setConfig({ ...config, private_key: e.target.value })} className={`${inputClass} min-h-36 font-mono text-xs`} placeholder="-----BEGIN PRIVATE KEY-----" /></div>
                </div>
              )}
              <div className="rounded-lg bg-blue-50 dark:bg-blue-900/20 p-3 text-sm text-blue-800 dark:text-blue-300">
                TransIP supports token auth and login/private-key auth. TXT support is required for DNS-01 and wildcard Let’s Encrypt certificates.
              </div>
            </div>
          )}
          {provider !== 'transip' && (
            <div><label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">Token / API Key</label><input type="password" value={config.token || ''} onChange={(e) => setConfig({ ...config, token: e.target.value })} className={inputClass} placeholder={profile ? 'Leave as-is or replace' : ''} /></div>
          )}
          <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
            {(['a_records', 'aaaa_records', 'cname_records', 'txt_records', 'wildcard_records'] as const).map((key) => (
              <label key={key} className="flex items-center gap-2 text-sm text-gray-700 dark:text-gray-300"><input type="checkbox" checked={capabilities[key]} onChange={(e) => setCapabilities({ ...capabilities, [key]: e.target.checked })} /> {key.replace(/_/g, ' ')}</label>
            ))}
            <label className="flex items-center gap-2 text-sm text-gray-700 dark:text-gray-300"><input type="checkbox" checked={enabled} onChange={(e) => setEnabled(e.target.checked)} /> Enabled</label>
          </div>
          <div className="rounded-lg bg-yellow-50 dark:bg-yellow-900/20 p-3 text-sm text-yellow-800 dark:text-yellow-300">Wildcard Let’s Encrypt certificates require TXT record support for DNS-01 challenges.</div>
          <div className="flex justify-end gap-3 pt-4 border-t border-gray-200 dark:border-gray-700"><button type="button" onClick={onClose} className="px-4 py-2 text-gray-700 dark:text-gray-300 hover:bg-gray-100 dark:hover:bg-gray-700 rounded-lg">Cancel</button><button type="submit" disabled={saving} className="px-4 py-2 bg-blue-600 hover:bg-blue-700 disabled:opacity-50 text-white rounded-lg">{saving ? 'Saving...' : 'Save Profile'}</button></div>
        </form>
      </div>
    </div>
  );
}
