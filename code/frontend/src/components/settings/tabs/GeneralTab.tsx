import type { Dispatch, ReactNode, SetStateAction } from 'react';
import type { Setting } from '../../../api/settings';
import type { OAuthProvider } from '../../../api/oauth';
import { SettingsTabLayout } from './SettingsTabLayout';

type OAuthProviderConfig = { client_id: string; client_secret: string };

export type SettingsStatus = 'loading' | 'ready' | 'error';

type GeneralTabProps = {
  systemSettings: Record<string, Setting>;
  /** Whether `systemSettings` actually holds loaded values yet. Without this,
   *  an empty map renders as a confident "everything off". */
  settingsStatus: SettingsStatus;
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
  settingsStatus,
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

          {settingsStatus === 'error' && (
            <div className="mb-4 rounded-md bg-red-50 dark:bg-red-900/20 border border-red-200 dark:border-red-800 px-3 py-2 text-sm text-red-700 dark:text-red-300">
              These settings could not be loaded, so their current values are unknown.
            </div>
          )}

          <div className="border-b border-gray-200 dark:border-gray-700">
            <SettingSwitch
              label="Allow Open Registration"
              description="When disabled, users can only register using invite links."
              settingKey="registration_enabled"
              systemSettings={systemSettings}
              settingsStatus={settingsStatus}
              savingSettings={savingSettings}
              onToggleSetting={onToggleSetting}
            />
          </div>

          <SettingSwitch
            label="Require Admin Approval"
            description="New registrations require admin approval before users can log in. Users with invite links are auto-approved."
            settingKey="registration_require_approval"
            systemSettings={systemSettings}
            settingsStatus={settingsStatus}
            savingSettings={savingSettings}
            onToggleSetting={onToggleSetting}
          />
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
                  role="switch"
                  aria-label={`${provider.name} OAuth`}
                  aria-checked={provider.enabled}
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

type SettingSwitchProps = {
  label: string;
  description: string;
  settingKey: string;
  systemSettings: Record<string, Setting>;
  settingsStatus: SettingsStatus;
  savingSettings: boolean;
  onToggleSetting: (key: string) => void;
};

/** One registration toggle.
 *
 *  Both switches used to read `systemSettings[key]?.value === 'true'` directly,
 *  which collapses "loaded and off" and "never loaded" into the same rendering.
 *  Until the values are in, this reports aria-checked="mixed" and refuses
 *  clicks, so a failed fetch can never be mistaken for a real "off".
 */
function SettingSwitch({
  label,
  description,
  settingKey,
  systemSettings,
  settingsStatus,
  savingSettings,
  onToggleSetting,
}: SettingSwitchProps) {
  const known = settingsStatus === 'ready';
  const checked = systemSettings[settingKey]?.value === 'true';
  const disabled = savingSettings || !known;

  return (
    <div className="flex items-center justify-between py-4">
      <div className="flex-1">
        <div className="font-medium text-gray-900 dark:text-white">{label}</div>
        <div className="text-sm text-gray-500 dark:text-gray-400 mt-1">{description}</div>
      </div>
      <button
        role="switch"
        aria-label={label}
        aria-checked={known ? checked : 'mixed'}
        onClick={() => onToggleSetting(settingKey)}
        disabled={disabled}
        className={`relative inline-flex h-6 w-11 items-center rounded-full transition-colors focus:outline-none focus:ring-2 focus:ring-blue-500 focus:ring-offset-2 focus:ring-offset-white dark:focus:ring-offset-gray-800 ${
          known && checked ? 'bg-blue-600' : 'bg-gray-300 dark:bg-gray-600'
        } ${disabled ? 'opacity-50 cursor-not-allowed' : ''}`}
      >
        <span
          className={`inline-block h-4 w-4 transform rounded-full bg-white transition-transform ${
            known && checked ? 'translate-x-6' : 'translate-x-1'
          }`}
        />
      </button>
    </div>
  );
}
