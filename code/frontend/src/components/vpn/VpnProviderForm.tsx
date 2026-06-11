import { useState, useEffect } from 'react';
import { X, Shield, Info } from 'lucide-react';
import {
  VpnProvider,
  VpnType,
  SupportedProvider,
  CreateVpnProviderRequest,
  UpdateVpnProviderRequest,
  vpnApi,
  getVpnTypeLabel,
} from '../../api/vpn';

interface VpnProviderFormProps {
  provider?: VpnProvider | null;
  onClose: () => void;
  onSave: () => void;
}

export function VpnProviderForm({
  provider,
  onClose,
  onSave,
}: VpnProviderFormProps) {
  const isEditing = !!provider;

  // Form state
  const [name, setName] = useState(provider?.name || '');
  const [vpnType, setVpnType] = useState<VpnType>(provider?.vpn_type || 'wireguard');
  const [serviceProvider, setServiceProvider] = useState(provider?.service_provider || 'custom');
  const [enabled, setEnabled] = useState(provider?.enabled ?? true);
  const [killSwitch, setKillSwitch] = useState(provider?.kill_switch ?? true);
  const [firewallSubnets, setFirewallSubnets] = useState(
    provider?.firewall_outbound_subnets || '10.0.0.0/8,172.16.0.0/12,192.168.0.0/16'
  );

  // WireGuard credentials
  const [wgPrivateKey, setWgPrivateKey] = useState('');
  const [wgAddresses, setWgAddresses] = useState('');
  const [wgPublicKey, setWgPublicKey] = useState('');
  const [wgEndpointIp, setWgEndpointIp] = useState('');
  const [wgEndpointPort, setWgEndpointPort] = useState('');

  // OpenVPN credentials
  const [ovpnUsername, setOvpnUsername] = useState('');
  const [ovpnPassword, setOvpnPassword] = useState('');
  const [ovpnCountries, setOvpnCountries] = useState('');
  const [ovpnCities, setOvpnCities] = useState('');

  // Supported providers
  const [supportedProviders, setSupportedProviders] = useState<SupportedProvider[]>([]);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Load supported providers
  useEffect(() => {
    vpnApi.listSupportedProviders().then(setSupportedProviders).catch(console.error);
  }, []);

  // Get VPN types supported by selected service provider
  const selectedServiceProvider = supportedProviders.find(p => p.id === serviceProvider);
  const supportedVpnTypes = selectedServiceProvider?.vpn_types || ['wireguard', 'openvpn'];

  // Reset VPN type if not supported
  useEffect(() => {
    if (!supportedVpnTypes.includes(vpnType)) {
      setVpnType(supportedVpnTypes[0] as VpnType);
    }
  }, [serviceProvider, supportedVpnTypes, vpnType]);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setError(null);
    setSaving(true);

    try {
      // Build credentials based on VPN type
      let credentials: Record<string, unknown>;
      if (vpnType === 'wireguard') {
        if (!wgPrivateKey) {
          throw new Error('WireGuard private key is required');
        }
        credentials = {
          private_key: wgPrivateKey,
          addresses: wgAddresses ? wgAddresses.split(',').map(a => a.trim()) : [],
          ...(wgPublicKey && { public_key: wgPublicKey }),
          ...(wgEndpointIp && { endpoint_ip: wgEndpointIp }),
          ...(wgEndpointPort && { endpoint_port: parseInt(wgEndpointPort, 10) }),
        };
      } else {
        if (!ovpnUsername || !ovpnPassword) {
          throw new Error('OpenVPN username and password are required');
        }
        credentials = {
          username: ovpnUsername,
          password: ovpnPassword,
          ...(ovpnCountries && { server_countries: ovpnCountries }),
          ...(ovpnCities && { server_cities: ovpnCities }),
        };
      }

      if (isEditing) {
        const updateData: UpdateVpnProviderRequest = {
          name,
          service_provider: serviceProvider,
          enabled,
          kill_switch: killSwitch,
          firewall_outbound_subnets: firewallSubnets,
        };
        // Only include credentials if they were filled in
        if (vpnType === 'wireguard' && wgPrivateKey) {
          updateData.credentials = credentials as any;
        } else if (vpnType === 'openvpn' && ovpnUsername && ovpnPassword) {
          updateData.credentials = credentials as any;
        }
        await vpnApi.updateProvider(provider!.id, updateData);
      } else {
        const createData: CreateVpnProviderRequest = {
          name,
          vpn_type: vpnType,
          service_provider: serviceProvider,
          credentials: credentials as any,
          enabled,
          kill_switch: killSwitch,
          firewall_outbound_subnets: firewallSubnets,
        };
        await vpnApi.createProvider(createData);
      }

      onSave();
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to save provider');
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
      <div className="bg-white dark:bg-gray-800 rounded-lg shadow-xl w-full max-w-lg max-h-[90vh] overflow-y-auto">
        {/* Header */}
        <div className="flex items-center justify-between p-4 border-b border-gray-200 dark:border-gray-700">
          <h2 className="text-lg font-semibold text-gray-900 dark:text-white flex items-center gap-2">
            <Shield size={20} className="text-blue-500" />
            {isEditing ? 'Edit VPN Provider' : 'Add VPN Provider'}
          </h2>
          <button
            onClick={onClose}
            className="p-2 hover:bg-gray-100 dark:hover:bg-gray-700 rounded-lg transition-colors"
          >
            <X size={20} className="text-gray-500" />
          </button>
        </div>

        <form onSubmit={handleSubmit} className="p-4 space-y-4">
          {error && (
            <div className="p-3 bg-red-50 dark:bg-red-900/20 text-red-700 dark:text-red-300 rounded-lg text-sm">
              {error}
            </div>
          )}

          {/* Basic Info */}
          <div>
            <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">
              Name
            </label>
            <input
              type="text"
              value={name}
              onChange={e => setName(e.target.value)}
              placeholder="My VPN"
              required
              className="w-full px-3 py-2 border border-gray-200 dark:border-gray-600 rounded-lg bg-white dark:bg-gray-700 text-gray-900 dark:text-white focus:ring-2 focus:ring-blue-500 focus:border-transparent"
            />
          </div>

          {/* Service Provider */}
          <div>
            <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">
              Service Provider
            </label>
            <select
              value={serviceProvider}
              onChange={e => setServiceProvider(e.target.value)}
              disabled={isEditing}
              className="w-full px-3 py-2 border border-gray-200 dark:border-gray-600 rounded-lg bg-white dark:bg-gray-700 text-gray-900 dark:text-white focus:ring-2 focus:ring-blue-500 focus:border-transparent disabled:opacity-50"
            >
              {supportedProviders.map(p => (
                <option key={p.id} value={p.id}>
                  {p.name}
                </option>
              ))}
            </select>
            {selectedServiceProvider && (
              <p className="mt-1 text-xs text-gray-500 dark:text-gray-400">
                {selectedServiceProvider.description}
              </p>
            )}
          </div>

          {/* VPN Type */}
          <div>
            <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">
              VPN Type
            </label>
            <div className="flex gap-4">
              {supportedVpnTypes.map(type => (
                <label key={type} className="flex items-center gap-2 cursor-pointer">
                  <input
                    type="radio"
                    name="vpnType"
                    value={type}
                    checked={vpnType === type}
                    onChange={e => setVpnType(e.target.value as VpnType)}
                    disabled={isEditing}
                    className="text-blue-600 focus:ring-blue-500"
                  />
                  <span className="text-gray-900 dark:text-white">
                    {getVpnTypeLabel(type as VpnType)}
                  </span>
                </label>
              ))}
            </div>
          </div>

          {/* WireGuard Credentials */}
          {vpnType === 'wireguard' && (
            <div className="space-y-3 p-3 bg-gray-50 dark:bg-gray-700/50 rounded-lg">
              <h4 className="font-medium text-gray-900 dark:text-white text-sm">
                WireGuard Configuration
              </h4>
              <div>
                <label className="block text-xs text-gray-600 dark:text-gray-400 mb-1">
                  Private Key *
                </label>
                <input
                  type="password"
                  value={wgPrivateKey}
                  onChange={e => setWgPrivateKey(e.target.value)}
                  placeholder={isEditing ? '(unchanged)' : 'Enter WireGuard private key'}
                  required={!isEditing}
                  className="w-full px-3 py-2 text-sm border border-gray-200 dark:border-gray-600 rounded-lg bg-white dark:bg-gray-700 text-gray-900 dark:text-white"
                />
              </div>
              <div>
                <label className="block text-xs text-gray-600 dark:text-gray-400 mb-1">
                  Addresses (comma-separated)
                </label>
                <input
                  type="text"
                  value={wgAddresses}
                  onChange={e => setWgAddresses(e.target.value)}
                  placeholder="10.2.0.2/32"
                  className="w-full px-3 py-2 text-sm border border-gray-200 dark:border-gray-600 rounded-lg bg-white dark:bg-gray-700 text-gray-900 dark:text-white"
                />
              </div>
              {serviceProvider === 'custom' && (
                <>
                  <div>
                    <label className="block text-xs text-gray-600 dark:text-gray-400 mb-1">
                      Server Public Key
                    </label>
                    <input
                      type="text"
                      value={wgPublicKey}
                      onChange={e => setWgPublicKey(e.target.value)}
                      placeholder="Server's public key"
                      className="w-full px-3 py-2 text-sm border border-gray-200 dark:border-gray-600 rounded-lg bg-white dark:bg-gray-700 text-gray-900 dark:text-white"
                    />
                  </div>
                  <div className="grid grid-cols-2 gap-2">
                    <div>
                      <label className="block text-xs text-gray-600 dark:text-gray-400 mb-1">
                        Endpoint IP
                      </label>
                      <input
                        type="text"
                        value={wgEndpointIp}
                        onChange={e => setWgEndpointIp(e.target.value)}
                        placeholder="1.2.3.4"
                        className="w-full px-3 py-2 text-sm border border-gray-200 dark:border-gray-600 rounded-lg bg-white dark:bg-gray-700 text-gray-900 dark:text-white"
                      />
                    </div>
                    <div>
                      <label className="block text-xs text-gray-600 dark:text-gray-400 mb-1">
                        Endpoint Port
                      </label>
                      <input
                        type="number"
                        value={wgEndpointPort}
                        onChange={e => setWgEndpointPort(e.target.value)}
                        placeholder="51820"
                        className="w-full px-3 py-2 text-sm border border-gray-200 dark:border-gray-600 rounded-lg bg-white dark:bg-gray-700 text-gray-900 dark:text-white"
                      />
                    </div>
                  </div>
                </>
              )}
            </div>
          )}

          {/* OpenVPN Credentials */}
          {vpnType === 'openvpn' && (
            <div className="space-y-3 p-3 bg-gray-50 dark:bg-gray-700/50 rounded-lg">
              <h4 className="font-medium text-gray-900 dark:text-white text-sm">
                OpenVPN Configuration
              </h4>
              <div>
                <label className="block text-xs text-gray-600 dark:text-gray-400 mb-1">
                  Username *
                </label>
                <input
                  type="text"
                  value={ovpnUsername}
                  onChange={e => setOvpnUsername(e.target.value)}
                  placeholder={isEditing ? '(unchanged)' : 'VPN username'}
                  required={!isEditing}
                  className="w-full px-3 py-2 text-sm border border-gray-200 dark:border-gray-600 rounded-lg bg-white dark:bg-gray-700 text-gray-900 dark:text-white"
                />
              </div>
              <div>
                <label className="block text-xs text-gray-600 dark:text-gray-400 mb-1">
                  Password *
                </label>
                <input
                  type="password"
                  value={ovpnPassword}
                  onChange={e => setOvpnPassword(e.target.value)}
                  placeholder={isEditing ? '(unchanged)' : 'VPN password'}
                  required={!isEditing}
                  className="w-full px-3 py-2 text-sm border border-gray-200 dark:border-gray-600 rounded-lg bg-white dark:bg-gray-700 text-gray-900 dark:text-white"
                />
              </div>
              {serviceProvider !== 'custom' && (
                <div className="grid grid-cols-2 gap-2">
                  <div>
                    <label className="block text-xs text-gray-600 dark:text-gray-400 mb-1">
                      Server Countries
                    </label>
                    <input
                      type="text"
                      value={ovpnCountries}
                      onChange={e => setOvpnCountries(e.target.value)}
                      placeholder="US,UK"
                      className="w-full px-3 py-2 text-sm border border-gray-200 dark:border-gray-600 rounded-lg bg-white dark:bg-gray-700 text-gray-900 dark:text-white"
                    />
                  </div>
                  <div>
                    <label className="block text-xs text-gray-600 dark:text-gray-400 mb-1">
                      Server Cities
                    </label>
                    <input
                      type="text"
                      value={ovpnCities}
                      onChange={e => setOvpnCities(e.target.value)}
                      placeholder="New York"
                      className="w-full px-3 py-2 text-sm border border-gray-200 dark:border-gray-600 rounded-lg bg-white dark:bg-gray-700 text-gray-900 dark:text-white"
                    />
                  </div>
                </div>
              )}
            </div>
          )}

          {/* Settings */}
          <div className="space-y-3">
            <div className="flex items-center justify-between">
              <div>
                <label className="font-medium text-gray-900 dark:text-white text-sm">
                  Kill Switch
                </label>
                <p className="text-xs text-gray-500 dark:text-gray-400">
                  Block traffic if VPN disconnects
                </p>
              </div>
              <button
                type="button"
                onClick={() => setKillSwitch(!killSwitch)}
                className={`relative inline-flex h-6 w-11 items-center rounded-full transition-colors ${
                  killSwitch ? 'bg-blue-600' : 'bg-gray-200 dark:bg-gray-600'
                }`}
              >
                <span
                  className={`inline-block h-4 w-4 transform rounded-full bg-white transition-transform ${
                    killSwitch ? 'translate-x-6' : 'translate-x-1'
                  }`}
                />
              </button>
            </div>

            <div className="flex items-center justify-between">
              <div>
                <label className="font-medium text-gray-900 dark:text-white text-sm">
                  Enabled
                </label>
                <p className="text-xs text-gray-500 dark:text-gray-400">
                  Provider available for use
                </p>
              </div>
              <button
                type="button"
                onClick={() => setEnabled(!enabled)}
                className={`relative inline-flex h-6 w-11 items-center rounded-full transition-colors ${
                  enabled ? 'bg-blue-600' : 'bg-gray-200 dark:bg-gray-600'
                }`}
              >
                <span
                  className={`inline-block h-4 w-4 transform rounded-full bg-white transition-transform ${
                    enabled ? 'translate-x-6' : 'translate-x-1'
                  }`}
                />
              </button>
            </div>
          </div>

          {/* Firewall Subnets */}
          <div>
            <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">
              Allowed Subnets
            </label>
            <input
              type="text"
              value={firewallSubnets}
              onChange={e => setFirewallSubnets(e.target.value)}
              placeholder="10.0.0.0/8,172.16.0.0/12,192.168.0.0/16"
              className="w-full px-3 py-2 border border-gray-200 dark:border-gray-600 rounded-lg bg-white dark:bg-gray-700 text-gray-900 dark:text-white"
            />
            <p className="mt-1 text-xs text-gray-500 dark:text-gray-400 flex items-center gap-1">
              <Info size={12} />
              Subnets allowed through firewall (for cluster communication)
            </p>
          </div>

          {/* Actions */}
          <div className="flex justify-end gap-3 pt-4 border-t border-gray-200 dark:border-gray-700">
            <button
              type="button"
              onClick={onClose}
              className="px-4 py-2 text-gray-700 dark:text-gray-300 hover:bg-gray-100 dark:hover:bg-gray-700 rounded-lg transition-colors"
            >
              Cancel
            </button>
            <button
              type="submit"
              disabled={saving}
              className="px-4 py-2 bg-blue-600 hover:bg-blue-700 text-white rounded-lg transition-colors disabled:opacity-50"
            >
              {saving ? 'Saving...' : isEditing ? 'Save Changes' : 'Add Provider'}
            </button>
          </div>
        </form>
      </div>
    </div>
  );
}
