import { useState, useEffect } from 'react';
import { Shield, ShieldCheck, ShieldOff, Trash2, Server, Plus } from 'lucide-react';
import {
  AppVpnConfig,
  VpnProvider,
  appVpnApi,
  AssignVpnRequest,
} from '../../api/vpn';
import { appsApi } from '../../api/apps';
import { AppIcon } from '../AppIcon';

interface AppVpnAssignmentsProps {
  configs: AppVpnConfig[];
  providers: VpnProvider[];
  onRefresh: () => void;
}

export function AppVpnAssignments({
  configs,
  providers,
  onRefresh,
}: AppVpnAssignmentsProps) {
  const [removingApp, setRemovingApp] = useState<string | null>(null);
  const [showAssignForm, setShowAssignForm] = useState(false);
  const [installedApps, setInstalledApps] = useState<string[]>([]);
  const [loadingApps, setLoadingApps] = useState(false);
  const [selectedApp, setSelectedApp] = useState<string>('');
  const [selectedProvider, setSelectedProvider] = useState<number | ''>('');
  const [killSwitchOverride, setKillSwitchOverride] = useState<boolean | null>(null);
  const [assigning, setAssigning] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Get list of apps that don't have VPN assigned yet
  const availableApps = installedApps.filter(
    app => !configs.some(c => c.app_name === app)
  );

  // Get enabled providers only
  const enabledProviders = providers.filter(p => p.enabled);

  useEffect(() => {
    if (showAssignForm && installedApps.length === 0) {
      loadInstalledApps();
    }
  }, [showAssignForm]);

  const loadInstalledApps = async () => {
    setLoadingApps(true);
    try {
      const apps = await appsApi.getInstalled();
      setInstalledApps(apps);
    } catch (err) {
      console.error('Failed to load installed apps:', err);
    } finally {
      setLoadingApps(false);
    }
  };

  const handleRemove = async (appName: string) => {
    if (!confirm(`Remove VPN from ${appName}? The app will be redeployed without the VPN sidecar.`)) {
      return;
    }
    setRemovingApp(appName);
    try {
      await appVpnApi.removeVpn(appName);
      onRefresh();
    } catch (error) {
      console.error('Failed to remove VPN:', error);
    } finally {
      setRemovingApp(null);
    }
  };

  const handleAssign = async () => {
    if (!selectedApp || !selectedProvider) return;

    setAssigning(true);
    setError(null);
    try {
      const request: AssignVpnRequest = {
        vpn_provider_id: selectedProvider as number,
      };
      if (killSwitchOverride !== null) {
        request.kill_switch_override = killSwitchOverride;
      }
      await appVpnApi.assignVpn(selectedApp, request);
      setShowAssignForm(false);
      setSelectedApp('');
      setSelectedProvider('');
      setKillSwitchOverride(null);
      onRefresh();
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to assign VPN');
    } finally {
      setAssigning(false);
    }
  };

  const handleCancel = () => {
    setShowAssignForm(false);
    setSelectedApp('');
    setSelectedProvider('');
    setKillSwitchOverride(null);
    setError(null);
  };

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <h3 className="text-lg font-semibold text-gray-900 dark:text-white">
          App VPN Assignments
        </h3>
        {enabledProviders.length > 0 && (
          <button
            onClick={() => setShowAssignForm(true)}
            className="flex items-center gap-2 px-4 py-2 bg-blue-600 hover:bg-blue-700 text-white rounded-lg transition-colors"
          >
            <Plus size={18} />
            Assign VPN to App
          </button>
        )}
      </div>

      {/* Assign Form */}
      {showAssignForm && (
        <div className="bg-white dark:bg-gray-800 rounded-lg border border-gray-200 dark:border-gray-700 p-6">
          <h4 className="text-md font-medium text-gray-900 dark:text-white mb-4">
            Assign VPN to App
          </h4>

          {loadingApps ? (
            <div className="flex items-center justify-center py-4">
              <div className="animate-spin rounded-full h-6 w-6 border-b-2 border-blue-500"></div>
            </div>
          ) : availableApps.length === 0 ? (
            <div className="text-center py-4">
              <p className="text-gray-500 dark:text-gray-400">
                {installedApps.length === 0
                  ? 'No apps installed. Install an app first.'
                  : 'All installed apps already have VPN assigned.'}
              </p>
            </div>
          ) : (
            <div className="space-y-4">
              {error && (
                <div className="p-3 bg-red-50 dark:bg-red-900/20 border border-red-200 dark:border-red-800 rounded-lg text-red-700 dark:text-red-300 text-sm">
                  {error}
                </div>
              )}

              {/* App Selection */}
              <div>
                <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-2">
                  Select App
                </label>
                <select
                  value={selectedApp}
                  onChange={(e) => setSelectedApp(e.target.value)}
                  className="w-full px-3 py-2 bg-gray-50 dark:bg-gray-900 border border-gray-200 dark:border-gray-700 rounded-lg text-gray-900 dark:text-white focus:outline-none focus:ring-2 focus:ring-blue-500"
                >
                  <option value="">Choose an app...</option>
                  {availableApps.map(app => (
                    <option key={app} value={app}>
                      {app.charAt(0).toUpperCase() + app.slice(1)}
                    </option>
                  ))}
                </select>
              </div>

              {/* Provider Selection */}
              <div>
                <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-2">
                  VPN Provider
                </label>
                <select
                  value={selectedProvider}
                  onChange={(e) => setSelectedProvider(e.target.value ? parseInt(e.target.value) : '')}
                  className="w-full px-3 py-2 bg-gray-50 dark:bg-gray-900 border border-gray-200 dark:border-gray-700 rounded-lg text-gray-900 dark:text-white focus:outline-none focus:ring-2 focus:ring-blue-500"
                >
                  <option value="">Choose a provider...</option>
                  {enabledProviders.map(provider => (
                    <option key={provider.id} value={provider.id}>
                      {provider.name} ({provider.vpn_type === 'wireguard' ? 'WireGuard' : 'OpenVPN'})
                    </option>
                  ))}
                </select>
              </div>

              {/* Kill Switch Override */}
              <div>
                <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-2">
                  Kill Switch
                </label>
                <select
                  value={killSwitchOverride === null ? '' : killSwitchOverride.toString()}
                  onChange={(e) => {
                    if (e.target.value === '') {
                      setKillSwitchOverride(null);
                    } else {
                      setKillSwitchOverride(e.target.value === 'true');
                    }
                  }}
                  className="w-full px-3 py-2 bg-gray-50 dark:bg-gray-900 border border-gray-200 dark:border-gray-700 rounded-lg text-gray-900 dark:text-white focus:outline-none focus:ring-2 focus:ring-blue-500"
                >
                  <option value="">Use provider default</option>
                  <option value="true">Force ON</option>
                  <option value="false">Force OFF</option>
                </select>
                <p className="mt-1 text-xs text-gray-500 dark:text-gray-400">
                  Kill switch blocks internet if VPN disconnects
                </p>
              </div>

              {/* Actions */}
              <div className="flex justify-end gap-3 pt-2">
                <button
                  onClick={handleCancel}
                  className="px-4 py-2 text-gray-700 dark:text-gray-300 hover:bg-gray-100 dark:hover:bg-gray-700 rounded-lg transition-colors"
                >
                  Cancel
                </button>
                <button
                  onClick={handleAssign}
                  disabled={!selectedApp || !selectedProvider || assigning}
                  className="flex items-center gap-2 px-4 py-2 bg-blue-600 hover:bg-blue-700 disabled:bg-blue-400 text-white rounded-lg transition-colors disabled:cursor-not-allowed"
                >
                  {assigning ? (
                    <>
                      <div className="w-4 h-4 border-2 border-white border-t-transparent rounded-full animate-spin" />
                      Assigning...
                    </>
                  ) : (
                    <>
                      <Shield size={18} />
                      Assign VPN
                    </>
                  )}
                </button>
              </div>

              <p className="text-xs text-gray-500 dark:text-gray-400 mt-2">
                The app will be automatically redeployed with the VPN sidecar.
              </p>
            </div>
          )}
        </div>
      )}

      {/* No Providers Warning */}
      {enabledProviders.length === 0 && providers.length > 0 && (
        <div className="bg-yellow-50 dark:bg-yellow-900/20 border border-yellow-200 dark:border-yellow-800 rounded-lg p-4">
          <p className="text-yellow-700 dark:text-yellow-300 text-sm">
            All VPN providers are disabled. Enable a provider to assign VPN to apps.
          </p>
        </div>
      )}

      {/* Assignments Table */}
      {configs.length === 0 ? (
        <div className="bg-white dark:bg-gray-800 rounded-lg border border-gray-200 dark:border-gray-700 p-8 text-center">
          <Server size={48} className="mx-auto mb-4 text-gray-400" />
          <h3 className="text-lg font-medium text-gray-900 dark:text-white mb-2">
            No Apps Using VPN
          </h3>
          <p className="text-gray-500 dark:text-gray-400">
            {providers.length === 0
              ? 'Add a VPN provider first, then assign it to apps.'
              : 'Click "Assign VPN to App" to route an app\'s traffic through VPN.'}
          </p>
        </div>
      ) : (
        <div className="bg-white dark:bg-gray-800 rounded-lg border border-gray-200 dark:border-gray-700 overflow-hidden">
          <table className="w-full">
            <thead>
              <tr className="border-b border-gray-200 dark:border-gray-700 bg-gray-50 dark:bg-gray-700/50">
                <th className="text-left px-4 py-3 text-sm font-medium text-gray-600 dark:text-gray-300">
                  App
                </th>
                <th className="text-left px-4 py-3 text-sm font-medium text-gray-600 dark:text-gray-300">
                  VPN Provider
                </th>
                <th className="text-center px-4 py-3 text-sm font-medium text-gray-600 dark:text-gray-300">
                  Kill Switch
                </th>
                <th className="text-right px-4 py-3 text-sm font-medium text-gray-600 dark:text-gray-300">
                  Actions
                </th>
              </tr>
            </thead>
            <tbody>
              {configs.map(config => {
                const provider = providers.find(p => p.id === config.vpn_provider_id);
                return (
                  <tr
                    key={config.app_name}
                    className="border-b border-gray-200 dark:border-gray-700/50 hover:bg-gray-50 dark:hover:bg-gray-700/30"
                  >
                    <td className="px-4 py-3">
                      <div className="flex items-center gap-3">
                        <AppIcon appName={config.app_name} size={32} />
                        <span className="font-medium text-gray-900 dark:text-white capitalize">
                          {config.app_name}
                        </span>
                      </div>
                    </td>
                    <td className="px-4 py-3">
                      <div className="flex items-center gap-2">
                        <Shield size={16} className="text-blue-500" />
                        <span className="text-gray-900 dark:text-white">
                          {config.vpn_provider_name}
                        </span>
                        {provider && !provider.enabled && (
                          <span className="px-2 py-0.5 bg-yellow-100 dark:bg-yellow-900/30 text-yellow-700 dark:text-yellow-300 rounded text-xs">
                            Disabled
                          </span>
                        )}
                      </div>
                    </td>
                    <td className="px-4 py-3 text-center">
                      {config.effective_kill_switch ? (
                        <span className="inline-flex items-center gap-1 text-green-500">
                          <ShieldCheck size={16} />
                          <span className="text-sm">ON</span>
                        </span>
                      ) : (
                        <span className="inline-flex items-center gap-1 text-yellow-500">
                          <ShieldOff size={16} />
                          <span className="text-sm">OFF</span>
                        </span>
                      )}
                      {config.kill_switch_override !== null && (
                        <span className="text-xs text-gray-400 ml-1">(override)</span>
                      )}
                    </td>
                    <td className="px-4 py-3 text-right">
                      <button
                        onClick={() => handleRemove(config.app_name)}
                        disabled={removingApp === config.app_name}
                        className="p-2 text-gray-500 hover:text-red-500 hover:bg-red-50 dark:hover:bg-red-900/20 rounded-lg transition-colors disabled:opacity-50"
                        title="Remove VPN"
                      >
                        {removingApp === config.app_name ? (
                          <div className="w-4 h-4 border-2 border-red-500 border-t-transparent rounded-full animate-spin" />
                        ) : (
                          <Trash2 size={18} />
                        )}
                      </button>
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      )}

      <p className="text-sm text-gray-500 dark:text-gray-400">
        Apps are automatically redeployed when VPN settings change.
      </p>
    </div>
  );
}
