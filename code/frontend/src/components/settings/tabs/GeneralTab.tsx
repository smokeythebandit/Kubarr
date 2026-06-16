import type { Dispatch, ReactNode, SetStateAction } from 'react';
import type { Setting } from '../../../api/settings';
import type { OAuthProvider } from '../../../api/oauth';
import { SettingsTabLayout } from './SettingsTabLayout';

type OAuthProviderConfig = { client_id: string; client_secret: string };

type GeneralTabProps = {
  systemSettings: Record<string, Setting>;
  savingSettings: boolean;
  oauthProviders: OAuthProvider[];
  editingOAuthProvider: string | null;
  oauthProviderConfig: OAuthProviderConfig;
  savingOAuthProvider: boolean;
  onToggleSetting: (key: string) => void;
  onToggleOAuthProvider: (providerId: string, enabled: boolean) => void;
  onSaveOAuthProvider: (providerId: string) => void;
  setEditingOAuthProvider: Dispatch<SetStateAction<string | null>>;
  setOauthProviderConfig: Dispatch<SetStateAction<OAuthProviderConfig>>;
  getOAuthProviderIcon: (providerId: string) => ReactNode;
};

export function GeneralTab({
  systemSettings,
  savingSettings,
  oauthProviders,
  editingOAuthProvider,
  oauthProviderConfig,
  savingOAuthProvider,
  onToggleSetting,
  onToggleOAuthProvider,
  onSaveOAuthProvider,
  setEditingOAuthProvider,
  setOauthProviderConfig,
  getOAuthProviderIcon,
}: GeneralTabProps) {
  return (
    <SettingsTabLayout
      title="General Settings"
      description="Configure general access management settings."
    >

      <div className="bg-white dark:bg-gray-800 rounded-lg border border-gray-200 dark:border-gray-700 p-6 space-y-6">
        <div>
          <h4 className="text-lg font-medium text-gray-900 dark:text-white mb-4">User Registration</h4>

          <div className="flex items-center justify-between py-4 border-b border-gray-200 dark:border-gray-700">
            <div className="flex-1">
              <div className="font-medium text-gray-900 dark:text-white">Allow Open Registration</div>
              <div className="text-sm text-gray-500 dark:text-gray-400 mt-1">
                When disabled, users can only register using invite links.
              </div>
            </div>
            <button
              onClick={() => onToggleSetting('registration_enabled')}
              disabled={savingSettings}
              className={`relative inline-flex h-6 w-11 items-center rounded-full transition-colors focus:outline-none focus:ring-2 focus:ring-blue-500 focus:ring-offset-2 focus:ring-offset-white dark:focus:ring-offset-gray-800 ${
                systemSettings.registration_enabled?.value === 'true'
                  ? 'bg-blue-600'
                  : 'bg-gray-300 dark:bg-gray-600'
              } ${savingSettings ? 'opacity-50 cursor-not-allowed' : ''}`}
            >
              <span
                className={`inline-block h-4 w-4 transform rounded-full bg-white transition-transform ${
                  systemSettings.registration_enabled?.value === 'true'
                    ? 'translate-x-6'
                    : 'translate-x-1'
                }`}
              />
            </button>
          </div>

          <div className="flex items-center justify-between py-4">
            <div className="flex-1">
              <div className="font-medium text-gray-900 dark:text-white">Require Admin Approval</div>
              <div className="text-sm text-gray-500 dark:text-gray-400 mt-1">
                New registrations require admin approval before users can log in. Users with invite links are auto-approved.
              </div>
            </div>
            <button
              onClick={() => onToggleSetting('registration_require_approval')}
              disabled={savingSettings}
              className={`relative inline-flex h-6 w-11 items-center rounded-full transition-colors focus:outline-none focus:ring-2 focus:ring-blue-500 focus:ring-offset-2 focus:ring-offset-white dark:focus:ring-offset-gray-800 ${
                systemSettings.registration_require_approval?.value === 'true'
                  ? 'bg-blue-600'
                  : 'bg-gray-300 dark:bg-gray-600'
              } ${savingSettings ? 'opacity-50 cursor-not-allowed' : ''}`}
            >
              <span
                className={`inline-block h-4 w-4 transform rounded-full bg-white transition-transform ${
                  systemSettings.registration_require_approval?.value === 'true'
                    ? 'translate-x-6'
                    : 'translate-x-1'
                }`}
              />
            </button>
          </div>
        </div>
      </div>

      <div className="bg-white dark:bg-gray-800 rounded-lg border border-gray-200 dark:border-gray-700 p-6 space-y-6">
        <div>
          <h4 className="text-lg font-medium text-gray-900 dark:text-white mb-2">OAuth Providers</h4>
          <p className="text-sm text-gray-500 dark:text-gray-400">
            Allow users to sign in with their Google or Microsoft accounts. Configure your OAuth app credentials below.
          </p>
        </div>

        <div className="space-y-4">
          {oauthProviders.map((provider) => (
            <div key={provider.id} className="border border-gray-200 dark:border-gray-700 rounded-lg p-4">
              <div className="flex items-center justify-between mb-4">
                <div className="flex items-center gap-3">
                  <div className={`p-2 rounded-lg ${provider.enabled ? 'bg-blue-100 dark:bg-blue-900/30' : 'bg-gray-100 dark:bg-gray-700'}`}>
                    {getOAuthProviderIcon(provider.id)}
                  </div>
                  <div>
                    <div className="font-medium text-gray-900 dark:text-white">{provider.name}</div>
                    <div className="text-sm text-gray-500 dark:text-gray-400">
                      {provider.enabled ? (
                        provider.client_id ? 'Configured' : 'Enabled but not configured'
                      ) : 'Disabled'}
                    </div>
                  </div>
                </div>
                <button
                  onClick={() => onToggleOAuthProvider(provider.id, !provider.enabled)}
                  disabled={!provider.client_id && !provider.enabled}
                  title={!provider.client_id && !provider.enabled ? 'Configure credentials first' : ''}
                  className={`relative inline-flex h-6 w-11 items-center rounded-full transition-colors focus:outline-none focus:ring-2 focus:ring-blue-500 ${
                    provider.enabled ? 'bg-blue-600' : 'bg-gray-300 dark:bg-gray-600'
                  } ${!provider.client_id && !provider.enabled ? 'opacity-50 cursor-not-allowed' : ''}`}
                >
                  <span
                    className={`inline-block h-4 w-4 transform rounded-full bg-white transition-transform ${
                      provider.enabled ? 'translate-x-6' : 'translate-x-1'
                    }`}
                  />
                </button>
              </div>

              {editingOAuthProvider === provider.id ? (
                <div className="mt-4 space-y-3 border-t border-gray-200 dark:border-gray-700 pt-4">
                  <div>
                    <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">
                      Client ID
                    </label>
                    <input
                      type="text"
                      value={oauthProviderConfig.client_id}
                      onChange={(e) => setOauthProviderConfig(prev => ({ ...prev, client_id: e.target.value }))}
                      placeholder={provider.client_id || 'Enter client ID'}
                      className="w-full px-3 py-2 bg-gray-50 dark:bg-gray-900 border border-gray-200 dark:border-gray-700 rounded-md text-gray-900 dark:text-white placeholder-gray-500 focus:outline-none focus:ring-2 focus:ring-blue-500"
                    />
                  </div>
                  <div>
                    <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">
                      Client Secret
                    </label>
                    <input
                      type="password"
                      value={oauthProviderConfig.client_secret}
                      onChange={(e) => setOauthProviderConfig(prev => ({ ...prev, client_secret: e.target.value }))}
                      placeholder={provider.has_secret ? '••••••••' : 'Enter client secret'}
                      className="w-full px-3 py-2 bg-gray-50 dark:bg-gray-900 border border-gray-200 dark:border-gray-700 rounded-md text-gray-900 dark:text-white placeholder-gray-500 focus:outline-none focus:ring-2 focus:ring-blue-500"
                    />
                    <p className="mt-1 text-xs text-gray-500 dark:text-gray-400">
                      Leave blank to keep existing secret
                    </p>
                  </div>
                  <div className="flex gap-2 mt-4">
                    <button
                      onClick={() => onSaveOAuthProvider(provider.id)}
                      disabled={savingOAuthProvider}
                      className="px-4 py-2 bg-blue-600 hover:bg-blue-700 disabled:bg-blue-400 text-white rounded-md font-medium transition-colors"
                    >
                      {savingOAuthProvider ? 'Saving...' : 'Save'}
                    </button>
                    <button
                      onClick={() => {
                        setEditingOAuthProvider(null);
                        setOauthProviderConfig({ client_id: '', client_secret: '' });
                      }}
                      className="px-4 py-2 bg-gray-200 dark:bg-gray-700 hover:bg-gray-300 dark:hover:bg-gray-600 text-gray-700 dark:text-gray-300 rounded-md font-medium transition-colors"
                    >
                      Cancel
                    </button>
                  </div>
                </div>
              ) : (
                <div className="mt-4 border-t border-gray-200 dark:border-gray-700 pt-4">
                  <button
                    onClick={() => {
                      setEditingOAuthProvider(provider.id);
                      setOauthProviderConfig({
                        client_id: provider.client_id || '',
                        client_secret: '',
                      });
                    }}
                    className="px-3 py-1.5 text-sm bg-gray-100 dark:bg-gray-700 hover:bg-gray-200 dark:hover:bg-gray-600 text-gray-700 dark:text-gray-300 rounded-md transition-colors"
                  >
                    Configure
                  </button>
                </div>
              )}
            </div>
          ))}

          {oauthProviders.length === 0 && (
            <p className="text-sm text-gray-500 dark:text-gray-400 italic">
              Loading OAuth providers...
            </p>
          )}
        </div>
      </div>
    </SettingsTabLayout>
  );
}
