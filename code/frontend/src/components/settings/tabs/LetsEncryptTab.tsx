import { useEffect, useMemo, useState } from 'react';
import type { FormEvent } from 'react';
import { Lock, Pencil, Plus, Trash2, X } from 'lucide-react';
import { domainsApi, type DynamicDnsProfile, type LetsEncryptProfile, type LetsEncryptProfileRequest } from '../../../api/domains';
import { SettingsCard, SettingsTabLayout } from './SettingsTabLayout';

const inputClass = 'w-full px-3 py-2 border border-gray-200 dark:border-gray-600 rounded-lg bg-white dark:bg-gray-700 text-gray-900 dark:text-white focus:ring-2 focus:ring-blue-500 focus:border-transparent';

export function LetsEncryptTab() {
  const [profiles, setProfiles] = useState<LetsEncryptProfile[]>([]);
  const [dnsProfiles, setDnsProfiles] = useState<DynamicDnsProfile[]>([]);
  const [editing, setEditing] = useState<LetsEncryptProfile | null>(null);
  const [showForm, setShowForm] = useState(false);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const loadData = async () => {
    try {
      setLoading(true);
      const [le, dns] = await Promise.all([domainsApi.listLetsEncryptProfiles(), domainsApi.listDdnsProfiles()]);
      setProfiles(le);
      setDnsProfiles(dns);
      setError(null);
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to load Let’s Encrypt profiles');
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => { loadData(); }, []);

  const handleDelete = async (profile: LetsEncryptProfile) => {
    if (!confirm(`Delete Let’s Encrypt profile "${profile.name}"?`)) return;
    await domainsApi.deleteLetsEncryptProfile(profile.id);
    await loadData();
  };

  return (
    <SettingsTabLayout
      title="Let’s Encrypt"
      description="Manage certificate automation profiles. Wildcard certificates require DNS-01 and a DNS profile with TXT support."
      actions={<button onClick={() => { setEditing(null); setShowForm(true); }} className="inline-flex items-center gap-2 px-4 py-2 bg-blue-600 hover:bg-blue-700 text-white rounded-lg transition-colors"><Plus size={18} /> Add Profile</button>}
    >
      {error && <div className="p-3 bg-red-50 dark:bg-red-900/20 text-red-700 dark:text-red-300 rounded-lg text-sm">{error}</div>}
      {loading ? <div className="py-8 text-center text-gray-500">Loading profiles...</div> : profiles.length === 0 ? (
        <SettingsCard><div className="py-8 text-center text-gray-500 dark:text-gray-400">No Let’s Encrypt profiles configured.</div></SettingsCard>
      ) : (
        <div className="grid gap-4">
          {profiles.map((profile) => {
            const dns = dnsProfiles.find((item) => item.id === profile.dns_profile_id);
            return (
              <SettingsCard key={profile.id}>
                <div className="flex items-start justify-between gap-4">
                  <div className="flex items-start gap-3">
                    <div className="w-10 h-10 rounded-lg bg-green-100 dark:bg-green-900/30 flex items-center justify-center"><Lock className="text-green-600 dark:text-green-300" size={22} /></div>
                    <div>
                      <div className="font-medium text-gray-900 dark:text-white">{profile.name}</div>
                      <div className="mt-1 flex flex-wrap gap-2 text-xs">
                        <span className="px-2 py-0.5 rounded bg-gray-100 dark:bg-gray-700 text-gray-600 dark:text-gray-300">{profile.environment}</span>
                        <span className="px-2 py-0.5 rounded bg-blue-100 dark:bg-blue-900/30 text-blue-700 dark:text-blue-300">{profile.challenge_type.toUpperCase()}</span>
                        {dns && <span className="px-2 py-0.5 rounded bg-purple-100 dark:bg-purple-900/30 text-purple-700 dark:text-purple-300">DNS: {dns.name}</span>}
                        <span className={profile.enabled ? 'text-green-600 dark:text-green-400' : 'text-gray-500'}>{profile.enabled ? 'Enabled' : 'Disabled'}</span>
                      </div>
                    </div>
                  </div>
                  <div className="flex gap-2"><button onClick={() => { setEditing(profile); setShowForm(true); }} className="p-2 text-gray-500 hover:text-blue-500 hover:bg-blue-50 dark:hover:bg-blue-900/20 rounded-lg"><Pencil size={18} /></button><button onClick={() => handleDelete(profile)} className="p-2 text-gray-500 hover:text-red-500 hover:bg-red-50 dark:hover:bg-red-900/20 rounded-lg"><Trash2 size={18} /></button></div>
                </div>
              </SettingsCard>
            );
          })}
        </div>
      )}
      {showForm && <LetsEncryptProfileModal profile={editing} dnsProfiles={dnsProfiles} onClose={() => setShowForm(false)} onSave={async () => { setShowForm(false); await loadData(); }} />}
    </SettingsTabLayout>
  );
}

function LetsEncryptProfileModal({ profile, dnsProfiles, onClose, onSave }: { profile: LetsEncryptProfile | null; dnsProfiles: DynamicDnsProfile[]; onClose: () => void; onSave: () => Promise<void> }) {
  const txtDnsProfiles = useMemo(() => dnsProfiles.filter((item) => item.capabilities.txt_records), [dnsProfiles]);
  const [name, setName] = useState(profile?.name || '');
  const [email, setEmail] = useState(profile?.email || '');
  const [environment, setEnvironment] = useState(profile?.environment || 'staging');
  const [challengeType, setChallengeType] = useState(profile?.challenge_type || 'http01');
  const [dnsProfileId, setDnsProfileId] = useState<number | ''>(profile?.dns_profile_id || '');
  const [renewalEnabled, setRenewalEnabled] = useState(profile?.renewal_enabled ?? true);
  const [enabled, setEnabled] = useState(profile?.enabled ?? true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const submit = async (event: FormEvent) => {
    event.preventDefault();
    if (challengeType === 'dns01' && !dnsProfileId) {
      setError('DNS-01 requires a Dynamic DNS profile with TXT record support.');
      return;
    }
    const request: LetsEncryptProfileRequest = { name, email, environment, challenge_type: challengeType, dns_profile_id: dnsProfileId || null, renewal_enabled: renewalEnabled, enabled };
    try {
      setSaving(true);
      if (profile) await domainsApi.updateLetsEncryptProfile(profile.id, request);
      else await domainsApi.createLetsEncryptProfile(request);
      await onSave();
    } catch (err: any) {
      setError(err.response?.data?.detail || 'Failed to save Let’s Encrypt profile');
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50 p-4">
      <div className="bg-white dark:bg-gray-800 rounded-lg shadow-xl w-full max-w-xl max-h-[90vh] overflow-y-auto">
        <div className="flex items-center justify-between p-4 border-b border-gray-200 dark:border-gray-700"><h2 className="text-lg font-semibold text-gray-900 dark:text-white">{profile ? 'Edit Let’s Encrypt Profile' : 'Add Let’s Encrypt Profile'}</h2><button onClick={onClose} className="p-2 hover:bg-gray-100 dark:hover:bg-gray-700 rounded-lg"><X size={20} /></button></div>
        <form onSubmit={submit} className="p-4 space-y-4">
          {error && <div className="p-3 bg-red-50 dark:bg-red-900/20 text-red-700 dark:text-red-300 rounded-lg text-sm">{error}</div>}
          <div><label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">Name</label><input required value={name} onChange={(e) => setName(e.target.value)} className={inputClass} placeholder="Production DNS-01" /></div>
          <div><label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">Account Email</label><input required type="email" value={email} onChange={(e) => setEmail(e.target.value)} className={inputClass} placeholder="admin@example.com" /></div>
          <div className="grid grid-cols-1 sm:grid-cols-2 gap-3"><div><label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">Environment</label><select value={environment} onChange={(e) => setEnvironment(e.target.value)} className={inputClass}><option value="staging">Staging</option><option value="production">Production</option></select></div><div><label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">Challenge Type</label><select value={challengeType} onChange={(e) => setChallengeType(e.target.value)} className={inputClass}><option value="http01">HTTP-01</option><option value="dns01">DNS-01</option></select></div></div>
          {challengeType === 'dns01' && <div><label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">DNS Profile</label><select value={dnsProfileId} onChange={(e) => setDnsProfileId(e.target.value ? Number(e.target.value) : '')} className={inputClass}><option value="">Select TXT-capable DNS profile...</option>{txtDnsProfiles.map((dns) => <option key={dns.id} value={dns.id}>{dns.name}</option>)}</select><p className="mt-1 text-xs text-gray-500 dark:text-gray-400">Required for wildcard certificates and any DNS-01 certificate.</p></div>}
          <div className="grid grid-cols-1 sm:grid-cols-2 gap-3"><label className="flex items-center gap-2 text-sm text-gray-700 dark:text-gray-300"><input type="checkbox" checked={renewalEnabled} onChange={(e) => setRenewalEnabled(e.target.checked)} /> Automatic renewal</label><label className="flex items-center gap-2 text-sm text-gray-700 dark:text-gray-300"><input type="checkbox" checked={enabled} onChange={(e) => setEnabled(e.target.checked)} /> Enabled</label></div>
          <div className="rounded-lg bg-blue-50 dark:bg-blue-900/20 p-3 text-sm text-blue-800 dark:text-blue-300">Use staging first to avoid Let’s Encrypt production rate limits. Wildcard certificates must use DNS-01.</div>
          <div className="flex justify-end gap-3 pt-4 border-t border-gray-200 dark:border-gray-700"><button type="button" onClick={onClose} className="px-4 py-2 text-gray-700 dark:text-gray-300 hover:bg-gray-100 dark:hover:bg-gray-700 rounded-lg">Cancel</button><button type="submit" disabled={saving} className="px-4 py-2 bg-blue-600 hover:bg-blue-700 disabled:opacity-50 text-white rounded-lg">{saving ? 'Saving...' : 'Save Profile'}</button></div>
        </form>
      </div>
    </div>
  );
}
