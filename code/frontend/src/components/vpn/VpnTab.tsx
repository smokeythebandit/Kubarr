import { useState, useEffect } from 'react';
import { Shield, RefreshCw, AlertCircle } from 'lucide-react';
import { VpnProvider, AppVpnConfig, vpnApi, appVpnApi } from '../../api/vpn';
import { VpnProviderList } from './VpnProviderList';
import { VpnProviderForm } from './VpnProviderForm';
import { AppVpnAssignments } from './AppVpnAssignments';

export function VpnTab() {
  const [providers, setProviders] = useState<VpnProvider[]>([]);
  const [appConfigs, setAppConfigs] = useState<AppVpnConfig[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [showForm, setShowForm] = useState(false);
  const [editingProvider, setEditingProvider] = useState<VpnProvider | null>(null);

  const fetchData = async () => {
    setLoading(true);
    setError(null);
    try {
      const [providersData, configsData] = await Promise.all([
        vpnApi.listProviders(),
        appVpnApi.listConfigs(),
      ]);
      setProviders(providersData);
      setAppConfigs(configsData);
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to load VPN data');
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    fetchData();
  }, []);

  const handleAddProvider = () => {
    setEditingProvider(null);
    setShowForm(true);
  };

  const handleEditProvider = (provider: VpnProvider) => {
    setEditingProvider(provider);
    setShowForm(true);
  };

  const handleFormClose = () => {
    setShowForm(false);
    setEditingProvider(null);
  };

  const handleFormSave = () => {
    handleFormClose();
    fetchData();
  };

  if (loading) {
    return (
      <div className="flex items-center justify-center py-12">
        <div className="animate-spin rounded-full h-8 w-8 border-b-2 border-blue-500"></div>
      </div>
    );
  }

  if (error) {
    return (
      <div className="flex flex-col items-center justify-center py-12 text-center">
        <AlertCircle size={48} className="text-red-500 mb-4" />
        <h3 className="text-lg font-medium text-gray-900 dark:text-white mb-2">
          Error Loading VPN Data
        </h3>
        <p className="text-gray-500 dark:text-gray-400 mb-4">{error}</p>
        <button
          onClick={fetchData}
          className="inline-flex items-center gap-2 px-4 py-2 bg-blue-600 hover:bg-blue-700 text-white rounded-lg transition-colors"
        >
          <RefreshCw size={18} />
          Retry
        </button>
      </div>
    );
  }

  return (
    <div className="space-y-8">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div>
          <h2 className="text-xl font-semibold flex items-center gap-2 text-gray-900 dark:text-white">
            <Shield size={20} className="text-blue-500" />
            VPN Configuration
          </h2>
          <p className="text-sm text-gray-500 dark:text-gray-400 mt-1">
            Route app traffic through VPN using Gluetun sidecars
          </p>
        </div>
        <button
          onClick={fetchData}
          className="flex items-center gap-2 px-4 py-2 text-gray-700 dark:text-gray-300 hover:bg-gray-100 dark:hover:bg-gray-700 rounded-lg transition-colors"
        >
          <RefreshCw size={18} />
          Refresh
        </button>
      </div>

      {/* Providers */}
      <VpnProviderList
        providers={providers}
        onRefresh={fetchData}
        onEdit={handleEditProvider}
        onAdd={handleAddProvider}
      />

      {/* App Assignments */}
      <AppVpnAssignments
        configs={appConfigs}
        providers={providers}
        onRefresh={fetchData}
      />

      {/* Provider Form Modal */}
      {showForm && (
        <VpnProviderForm
          provider={editingProvider}
          onClose={handleFormClose}
          onSave={handleFormSave}
        />
      )}
    </div>
  );
}
