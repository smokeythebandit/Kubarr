import { useEffect, useMemo, useState } from 'react';
import type { FormEvent, ReactNode } from 'react';
import { Globe, Link2, Pencil, Plus, Trash2, X } from 'lucide-react';
import type { App } from '../../../api/apps';
import { domainsApi, type AppDomainAssignment, type AppDomainAssignmentRequest, type DomainConfig, type DomainRequest, type DynamicDnsProfile, type LetsEncryptProfile } from '../../../api/domains';
import { SettingsCard, SettingsTabLayout } from './SettingsTabLayout';

type DomainsTabProps = { apps: App[] };
const inputClass = 'w-full px-3 py-2 border border-gray-200 dark:border-gray-600 rounded-lg bg-white dark:bg-gray-700 text-gray-900 dark:text-white focus:ring-2 focus:ring-blue-500 focus:border-transparent';

export function DomainsTab({ apps }: DomainsTabProps) {
  const [domains, setDomains] = useState<DomainConfig[]>([]);
  const [assignments, setAssignments] = useState<AppDomainAssignment[]>([]);
  const [ddnsProfiles, setDdnsProfiles] = useState<DynamicDnsProfile[]>([]);
  const [letsencryptProfiles, setLetsencryptProfiles] = useState<LetsEncryptProfile[]>([]);
  const [editingDomain, setEditingDomain] = useState<DomainConfig | null>(null);
  const [editingAssignment, setEditingAssignment] = useState<AppDomainAssignment | null>(null);
  const [showDomainForm, setShowDomainForm] = useState(false);
  const [showAssignmentForm, setShowAssignmentForm] = useState(false);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const browsableApps = useMemo(() => apps.filter((app) => app.is_browseable && !app.is_hidden).sort((a, b) => a.display_name.localeCompare(b.display_name)), [apps]);

  const loadData = async () => {
    try {
      setLoading(true);
      const [domainData, assignmentData, ddnsData, leData] = await Promise.all([
        domainsApi.listDomains(),
        domainsApi.listAssignments(),
        domainsApi.listDdnsProfiles(),
        domainsApi.listLetsEncryptProfiles(),
      ]);
      setDomains(domainData);
      setAssignments(assignmentData);
      setDdnsProfiles(ddnsData);
      setLetsencryptProfiles(leData);
      setError(null);
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to load domains');
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => { loadData(); }, []);

  const deleteDomain = async (domain: DomainConfig) => {
    if (!confirm(`Delete domain "${domain.domain}"? App URL assignments using it will be removed.`)) return;
    await domainsApi.deleteDomain(domain.id);
    await loadData();
  };

  const deleteAssignment = async (assignment: AppDomainAssignment) => {
    if (!confirm('Delete this app URL assignment?')) return;
    await domainsApi.deleteAssignment(assignment.id);
    await loadData();
  };

  const appName = (name: string) => browsableApps.find((app) => app.name === name)?.display_name || name;
  const domainName = (id: number) => domains.find((domain) => domain.id === id)?.domain || 'Unknown domain';

  return (
    <SettingsTabLayout
      title="Domains"
      description="Manage domain inventory and assign one or more public URLs to apps. DNS and certificate profiles live in their own panes."
      actions={<><button onClick={() => { setEditingAssignment(null); setShowAssignmentForm(true); }} disabled={domains.length === 0} className="inline-flex items-center gap-2 px-4 py-2 bg-gray-100 dark:bg-gray-700 hover:bg-gray-200 dark:hover:bg-gray-600 disabled:opacity-50 text-gray-700 dark:text-gray-200 rounded-lg transition-colors"><Link2 size={18} /> Assign App URL</button><button onClick={() => { setEditingDomain(null); setShowDomainForm(true); }} className="inline-flex items-center gap-2 px-4 py-2 bg-blue-600 hover:bg-blue-700 text-white rounded-lg transition-colors"><Plus size={18} /> Add Domain</button></>}
    >
      {error && <div className="p-3 bg-red-50 dark:bg-red-900/20 text-red-700 dark:text-red-300 rounded-lg text-sm">{error}</div>}
      {loading ? <div className="py-8 text-center text-gray-500">Loading domains...</div> : (
        <>
          <SettingsCard className="space-y-4">
            <div><h4 className="text-lg font-medium text-gray-900 dark:text-white">Domain Inventory</h4><p className="text-sm text-gray-500 dark:text-gray-400 mt-1">Root, wildcard, and exact domains Kubarr can use for routing.</p></div>
            {domains.length === 0 ? <div className="py-8 text-center text-gray-500 dark:text-gray-400 border border-dashed border-gray-300 dark:border-gray-700 rounded-lg">No domains configured yet.</div> : (
              <div className="grid gap-4">
                {domains.map((domain) => {
                  const ddns = ddnsProfiles.find((profile) => profile.id === domain.ddns_profile_id);
                  const le = letsencryptProfiles.find((profile) => profile.id === domain.letsencrypt_profile_id);
                  const count = assignments.filter((assignment) => assignment.domain_id === domain.id).length;
                  return (
                    <div key={domain.id} className="rounded-lg border border-gray-200 dark:border-gray-700 p-4">
                      <div className="flex items-start justify-between gap-4">
                        <div className="flex items-start gap-3"><div className="w-10 h-10 rounded-lg bg-blue-100 dark:bg-blue-900/30 flex items-center justify-center"><Globe className="text-blue-600 dark:text-blue-300" size={22} /></div><div><div className="font-medium text-gray-900 dark:text-white">{domain.domain}</div><div className="mt-1 flex flex-wrap gap-2 text-xs"><span className="px-2 py-0.5 rounded bg-gray-100 dark:bg-gray-700 text-gray-600 dark:text-gray-300">{domain.kind}</span><span className="px-2 py-0.5 rounded bg-gray-100 dark:bg-gray-700 text-gray-600 dark:text-gray-300">{domain.scope}</span>{domain.primary && <span className="px-2 py-0.5 rounded bg-blue-100 dark:bg-blue-900/30 text-blue-700 dark:text-blue-300">Primary</span>}<span>{count} app URL{count === 1 ? '' : 's'}</span></div><div className="mt-2 text-sm text-gray-500 dark:text-gray-400">DNS: {ddns ? ddns.name : domain.dns_mode} · TLS: {le ? le.name : domain.tls_mode}</div></div></div>
                        <div className="flex gap-2"><button onClick={() => { setEditingDomain(domain); setShowDomainForm(true); }} className="p-2 text-gray-500 hover:text-blue-500 hover:bg-blue-50 dark:hover:bg-blue-900/20 rounded-lg"><Pencil size={18} /></button><button onClick={() => deleteDomain(domain)} className="p-2 text-gray-500 hover:text-red-500 hover:bg-red-50 dark:hover:bg-red-900/20 rounded-lg"><Trash2 size={18} /></button></div>
                      </div>
                    </div>
                  );
                })}
              </div>
            )}
          </SettingsCard>

          <SettingsCard className="space-y-4">
            <div><h4 className="text-lg font-medium text-gray-900 dark:text-white">App URLs</h4><p className="text-sm text-gray-500 dark:text-gray-400 mt-1">Assign path, subdomain, or exact-host routes. Multiple URLs per app are supported.</p></div>
            {assignments.length === 0 ? <div className="py-8 text-center text-gray-500 dark:text-gray-400 border border-dashed border-gray-300 dark:border-gray-700 rounded-lg">No app URLs assigned yet.</div> : (
              <div className="grid gap-3">
                {assignments.map((assignment) => (
                  <div key={assignment.id} className="flex items-center justify-between gap-4 rounded-lg border border-gray-200 dark:border-gray-700 p-4">
                    <div><div className="font-medium text-gray-900 dark:text-white">{appName(assignment.app_name)}</div><div className="text-sm text-gray-500 dark:text-gray-400">{previewAssignment(assignment, domainName(assignment.domain_id))}</div><div className="mt-1 flex gap-2 text-xs"><span className="px-2 py-0.5 rounded bg-gray-100 dark:bg-gray-700 text-gray-600 dark:text-gray-300">{assignment.route_mode}</span>{assignment.primary && <span className="px-2 py-0.5 rounded bg-blue-100 dark:bg-blue-900/30 text-blue-700 dark:text-blue-300">Primary</span>}</div></div>
                    <div className="flex gap-2"><button onClick={() => { setEditingAssignment(assignment); setShowAssignmentForm(true); }} className="p-2 text-gray-500 hover:text-blue-500 hover:bg-blue-50 dark:hover:bg-blue-900/20 rounded-lg"><Pencil size={18} /></button><button onClick={() => deleteAssignment(assignment)} className="p-2 text-gray-500 hover:text-red-500 hover:bg-red-50 dark:hover:bg-red-900/20 rounded-lg"><Trash2 size={18} /></button></div>
                  </div>
                ))}
              </div>
            )}
          </SettingsCard>
        </>
      )}
      {showDomainForm && <DomainModal domain={editingDomain} ddnsProfiles={ddnsProfiles} letsencryptProfiles={letsencryptProfiles} onClose={() => setShowDomainForm(false)} onSave={async () => { setShowDomainForm(false); await loadData(); }} />}
      {showAssignmentForm && <AssignmentModal assignment={editingAssignment} apps={browsableApps} domains={domains} onClose={() => setShowAssignmentForm(false)} onSave={async () => { setShowAssignmentForm(false); await loadData(); }} />}
    </SettingsTabLayout>
  );
}

function DomainModal({ domain, ddnsProfiles, letsencryptProfiles, onClose, onSave }: { domain: DomainConfig | null; ddnsProfiles: DynamicDnsProfile[]; letsencryptProfiles: LetsEncryptProfile[]; onClose: () => void; onSave: () => Promise<void> }) {
  const [form, setForm] = useState<DomainRequest>({ domain: domain?.domain || '', kind: domain?.kind || 'root', scope: domain?.scope || 'public', primary: domain?.primary || false, enabled: domain?.enabled ?? true, dns_mode: domain?.dns_mode || 'manual', ddns_profile_id: domain?.ddns_profile_id || null, tls_mode: domain?.tls_mode || 'none', letsencrypt_profile_id: domain?.letsencrypt_profile_id || null, tls_secret_name: domain?.tls_secret_name || null });
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const compatibleLeProfiles = form.kind === 'wildcard' ? letsencryptProfiles.filter((profile) => profile.challenge_type === 'dns01') : letsencryptProfiles;

  const submit = async (event: FormEvent) => {
    event.preventDefault();
    if (form.kind === 'wildcard' && !form.domain.startsWith('*.')) { setError('Wildcard domains must start with *.'); return; }
    if (form.tls_mode === 'letsencrypt' && !form.letsencrypt_profile_id) { setError('Select a Let’s Encrypt profile.'); return; }
    try {
      setSaving(true);
      if (domain) await domainsApi.updateDomain(domain.id, form);
      else await domainsApi.createDomain(form);
      await onSave();
    } catch (err: any) {
      setError(err.response?.data?.detail || 'Failed to save domain');
    } finally { setSaving(false); }
  };

  return <Modal title={domain ? 'Edit Domain' : 'Add Domain'} onClose={onClose}><form onSubmit={submit} className="p-4 space-y-4">{error && <div className="p-3 bg-red-50 dark:bg-red-900/20 text-red-700 dark:text-red-300 rounded-lg text-sm">{error}</div>}<div><label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">Domain</label><input required value={form.domain} onChange={(e) => setForm({ ...form, domain: e.target.value })} className={inputClass} placeholder={form.kind === 'wildcard' ? '*.example.com' : 'example.com'} /></div><div className="grid grid-cols-1 sm:grid-cols-2 gap-3"><div><label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">Type</label><select value={form.kind} onChange={(e) => setForm({ ...form, kind: e.target.value, tls_mode: e.target.value === 'wildcard' && form.tls_mode === 'letsencrypt' && !compatibleLeProfiles.length ? 'none' : form.tls_mode })} className={inputClass}><option value="root">Root domain</option><option value="wildcard">Wildcard domain</option><option value="exact">Exact host</option></select></div><div><label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">Scope</label><select value={form.scope} onChange={(e) => setForm({ ...form, scope: e.target.value })} className={inputClass}><option value="public">Public</option><option value="private">Private/LAN</option><option value="both">Both</option></select></div></div><div className="grid grid-cols-1 sm:grid-cols-2 gap-3"><div><label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">DNS Mode</label><select value={form.dns_mode} onChange={(e) => setForm({ ...form, dns_mode: e.target.value, ddns_profile_id: e.target.value === 'manual' ? null : form.ddns_profile_id })} className={inputClass}><option value="manual">Manual/external</option><option value="dynamic_dns">Dynamic DNS profile</option></select></div>{form.dns_mode === 'dynamic_dns' && <div><label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">Dynamic DNS Profile</label><select value={form.ddns_profile_id || ''} onChange={(e) => setForm({ ...form, ddns_profile_id: e.target.value ? Number(e.target.value) : null })} className={inputClass}><option value="">Select profile...</option>{ddnsProfiles.map((profile) => <option key={profile.id} value={profile.id}>{profile.name}</option>)}</select></div>}</div><div className="grid grid-cols-1 sm:grid-cols-2 gap-3"><div><label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">TLS Mode</label><select value={form.tls_mode} onChange={(e) => setForm({ ...form, tls_mode: e.target.value, letsencrypt_profile_id: e.target.value === 'letsencrypt' ? form.letsencrypt_profile_id : null })} className={inputClass}><option value="none">None</option><option value="manual">Manual/external</option><option value="letsencrypt">Let’s Encrypt</option></select></div>{form.tls_mode === 'letsencrypt' && <div><label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">Let’s Encrypt Profile</label><select value={form.letsencrypt_profile_id || ''} onChange={(e) => setForm({ ...form, letsencrypt_profile_id: e.target.value ? Number(e.target.value) : null })} className={inputClass}><option value="">Select profile...</option>{compatibleLeProfiles.map((profile) => <option key={profile.id} value={profile.id}>{profile.name}</option>)}</select></div>}</div>{form.kind === 'wildcard' && <div className="rounded-lg bg-yellow-50 dark:bg-yellow-900/20 p-3 text-sm text-yellow-800 dark:text-yellow-300">Wildcard certificates require a DNS-01 Let’s Encrypt profile, which requires a Dynamic DNS profile with TXT record support.</div>}<div className="grid grid-cols-1 sm:grid-cols-2 gap-3"><label className="flex items-center gap-2 text-sm text-gray-700 dark:text-gray-300"><input type="checkbox" checked={form.primary} onChange={(e) => setForm({ ...form, primary: e.target.checked })} /> Primary domain</label><label className="flex items-center gap-2 text-sm text-gray-700 dark:text-gray-300"><input type="checkbox" checked={form.enabled} onChange={(e) => setForm({ ...form, enabled: e.target.checked })} /> Enabled</label></div><Actions saving={saving} onClose={onClose} /></form></Modal>;
}

function AssignmentModal({ assignment, apps, domains, onClose, onSave }: { assignment: AppDomainAssignment | null; apps: App[]; domains: DomainConfig[]; onClose: () => void; onSave: () => Promise<void> }) {
  const [form, setForm] = useState<AppDomainAssignmentRequest>({ app_name: assignment?.app_name || apps[0]?.name || '', domain_id: assignment?.domain_id || domains[0]?.id || 0, route_mode: assignment?.route_mode || 'subdomain', hostname: assignment?.hostname || '', path_prefix: assignment?.path_prefix || '', primary: assignment?.primary || false, enabled: assignment?.enabled ?? true });
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const selectedDomain = domains.find((domain) => domain.id === form.domain_id);
  const submit = async (event: FormEvent) => { event.preventDefault(); try { setSaving(true); if (assignment) await domainsApi.updateAssignment(assignment.id, form); else await domainsApi.createAssignment(form); await onSave(); } catch (err: any) { setError(err.response?.data?.detail || 'Failed to save app URL assignment'); } finally { setSaving(false); } };
  return <Modal title={assignment ? 'Edit App URL' : 'Assign App URL'} onClose={onClose}><form onSubmit={submit} className="p-4 space-y-4">{error && <div className="p-3 bg-red-50 dark:bg-red-900/20 text-red-700 dark:text-red-300 rounded-lg text-sm">{error}</div>}<div className="grid grid-cols-1 sm:grid-cols-2 gap-3"><div><label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">App</label><select value={form.app_name} onChange={(e) => setForm({ ...form, app_name: e.target.value })} className={inputClass}>{apps.map((app) => <option key={app.name} value={app.name}>{app.display_name}</option>)}</select></div><div><label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">Domain</label><select value={form.domain_id} onChange={(e) => setForm({ ...form, domain_id: Number(e.target.value) })} className={inputClass}>{domains.map((domain) => <option key={domain.id} value={domain.id}>{domain.domain}</option>)}</select></div></div><div><label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">Route Mode</label><select value={form.route_mode} onChange={(e) => setForm({ ...form, route_mode: e.target.value })} className={inputClass}><option value="path">Path: example.com/app</option><option value="subdomain">Subdomain: app.example.com</option><option value="exact_host">Exact host: custom.example.net</option></select></div>{form.route_mode === 'path' ? <div><label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">Path Prefix</label><input value={form.path_prefix || ''} onChange={(e) => setForm({ ...form, path_prefix: e.target.value })} className={inputClass} placeholder="/radarr" /></div> : <div><label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">Hostname or Subdomain</label><input value={form.hostname || ''} onChange={(e) => setForm({ ...form, hostname: e.target.value })} className={inputClass} placeholder={form.route_mode === 'subdomain' ? 'radarr' : 'radarr.example.com'} /></div>}<div className="rounded-lg bg-gray-50 dark:bg-gray-900 p-3 text-sm text-gray-700 dark:text-gray-300">Preview: {previewAssignment(form, selectedDomain?.domain || '')}</div><div className="grid grid-cols-1 sm:grid-cols-2 gap-3"><label className="flex items-center gap-2 text-sm text-gray-700 dark:text-gray-300"><input type="checkbox" checked={form.primary} onChange={(e) => setForm({ ...form, primary: e.target.checked })} /> Primary URL for app</label><label className="flex items-center gap-2 text-sm text-gray-700 dark:text-gray-300"><input type="checkbox" checked={form.enabled} onChange={(e) => setForm({ ...form, enabled: e.target.checked })} /> Enabled</label></div><Actions saving={saving} onClose={onClose} /></form></Modal>;
}

function Modal({ title, children, onClose }: { title: string; children: ReactNode; onClose: () => void }) {
  return <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50 p-4"><div className="bg-white dark:bg-gray-800 rounded-lg shadow-xl w-full max-w-2xl max-h-[90vh] overflow-y-auto"><div className="flex items-center justify-between p-4 border-b border-gray-200 dark:border-gray-700"><h2 className="text-lg font-semibold text-gray-900 dark:text-white">{title}</h2><button onClick={onClose} className="p-2 hover:bg-gray-100 dark:hover:bg-gray-700 rounded-lg"><X size={20} /></button></div>{children}</div></div>;
}

function Actions({ saving, onClose }: { saving: boolean; onClose: () => void }) {
  return <div className="flex justify-end gap-3 pt-4 border-t border-gray-200 dark:border-gray-700"><button type="button" onClick={onClose} className="px-4 py-2 text-gray-700 dark:text-gray-300 hover:bg-gray-100 dark:hover:bg-gray-700 rounded-lg">Cancel</button><button type="submit" disabled={saving} className="px-4 py-2 bg-blue-600 hover:bg-blue-700 disabled:opacity-50 text-white rounded-lg">{saving ? 'Saving...' : 'Save'}</button></div>;
}

function previewAssignment(assignment: Pick<AppDomainAssignment, 'route_mode' | 'hostname' | 'path_prefix'> | AppDomainAssignmentRequest, domain: string) {
  if (assignment.route_mode === 'path') return `https://${domain}${assignment.path_prefix || '/app'}`;
  if (assignment.route_mode === 'exact_host') return `https://${assignment.hostname || 'host.example.com'}`;
  const host = assignment.hostname || 'app';
  return `https://${host.includes('.') ? host : `${host}.${domain.replace(/^\*\./, '')}`}`;
}
