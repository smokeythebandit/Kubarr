import { useState } from 'react';
import {
  Shield,
  ShieldCheck,
  ShieldX,
  Plus,
  Pencil,
  Trash2,
  CheckCircle,
  XCircle,
  Server,
  Play,
} from 'lucide-react';
import {
  VpnProvider,
  vpnApi,
  getVpnTypeLabel,
  getProviderLabel,
} from '../../api/vpn';

interface VpnProviderListProps {
  providers: VpnProvider[];
  onRefresh: () => void;
  onEdit: (provider: VpnProvider) => void;
  onAdd: () => void;
}

export function VpnProviderList({
  providers,
  onRefresh,
  onEdit,
  onAdd,
}: VpnProviderListProps) {
  const [deletingId, setDeletingId] = useState<number | null>(null);
  const [testingId, setTestingId] = useState<number | null>(null);
  const [testResult, setTestResult] = useState<{
    id: number;
    success: boolean;
    message: string;
  } | null>(null);

  const handleDelete = async (id: number) => {
    if (!confirm('Delete this VPN provider? Apps using it will lose VPN connectivity.')) {
      return;
    }
    setDeletingId(id);
    try {
      await vpnApi.deleteProvider(id);
      onRefresh();
    } catch (error) {
      console.error('Failed to delete provider:', error);
    } finally {
      setDeletingId(null);
    }
  };

  const handleTest = async (id: number) => {
    setTestingId(id);
    setTestResult(null);
    try {
      const result = await vpnApi.testProvider(id);
      setTestResult({ id, ...result });
    } catch (error) {
      setTestResult({
        id,
        success: false,
        message: error instanceof Error ? error.message : 'Test failed',
      });
    } finally {
      setTestingId(null);
    }
  };

  if (providers.length === 0) {
    return (
      <div className="bg-white dark:bg-gray-800 rounded-lg border border-gray-200 dark:border-gray-700 p-8 text-center">
        <Shield size={48} className="mx-auto mb-4 text-gray-400" />
        <h3 className="text-lg font-medium text-gray-900 dark:text-white mb-2">
          No VPN Providers
        </h3>
        <p className="text-gray-500 dark:text-gray-400 mb-4">
          Add a VPN provider to route app traffic through a VPN tunnel.
        </p>
        <button
          onClick={onAdd}
          className="inline-flex items-center gap-2 px-4 py-2 bg-blue-600 hover:bg-blue-700 text-white rounded-lg transition-colors"
        >
          <Plus size={18} />
          Add VPN Provider
        </button>
      </div>
    );
  }

  return (
    <div className="space-y-4">
      <div className="flex justify-between items-center">
        <h3 className="text-lg font-semibold text-gray-900 dark:text-white">
          VPN Providers
        </h3>
        <button
          onClick={onAdd}
          className="inline-flex items-center gap-2 px-4 py-2 bg-blue-600 hover:bg-blue-700 text-white rounded-lg transition-colors"
        >
          <Plus size={18} />
          Add Provider
        </button>
      </div>

      <div className="grid gap-4">
        {providers.map((provider) => (
          <div
            key={provider.id}
            className="bg-white dark:bg-gray-800 rounded-lg border border-gray-200 dark:border-gray-700 p-4"
          >
            <div className="flex items-start justify-between">
              <div className="flex items-center gap-3">
                <div
                  className={`w-10 h-10 rounded-lg flex items-center justify-center ${
                    provider.enabled
                      ? 'bg-green-100 dark:bg-green-900/30'
                      : 'bg-gray-100 dark:bg-gray-700'
                  }`}
                >
                  {provider.enabled ? (
                    <ShieldCheck
                      size={24}
                      className="text-green-500 dark:text-green-400"
                    />
                  ) : (
                    <ShieldX
                      size={24}
                      className="text-gray-400 dark:text-gray-500"
                    />
                  )}
                </div>
                <div>
                  <h4 className="font-medium text-gray-900 dark:text-white">
                    {provider.name}
                  </h4>
                  <div className="flex items-center gap-2 text-sm text-gray-500 dark:text-gray-400">
                    <span className="px-2 py-0.5 bg-blue-100 dark:bg-blue-900/30 text-blue-700 dark:text-blue-300 rounded text-xs">
                      {getVpnTypeLabel(provider.vpn_type)}
                    </span>
                    {provider.service_provider && (
                      <span className="px-2 py-0.5 bg-gray-100 dark:bg-gray-700 text-gray-600 dark:text-gray-300 rounded text-xs">
                        {getProviderLabel(provider.service_provider)}
                      </span>
                    )}
                  </div>
                </div>
              </div>

              <div className="flex items-center gap-2">
                <button
                  onClick={() => handleTest(provider.id)}
                  disabled={testingId === provider.id}
                  className="p-2 text-gray-500 hover:text-blue-500 hover:bg-blue-50 dark:hover:bg-blue-900/20 rounded-lg transition-colors disabled:opacity-50"
                  title="Test connection"
                >
                  {testingId === provider.id ? (
                    <div className="w-4 h-4 border-2 border-blue-500 border-t-transparent rounded-full animate-spin" />
                  ) : (
                    <Play size={18} />
                  )}
                </button>
                <button
                  onClick={() => onEdit(provider)}
                  className="p-2 text-gray-500 hover:text-blue-500 hover:bg-blue-50 dark:hover:bg-blue-900/20 rounded-lg transition-colors"
                  title="Edit provider"
                >
                  <Pencil size={18} />
                </button>
                <button
                  onClick={() => handleDelete(provider.id)}
                  disabled={deletingId === provider.id}
                  className="p-2 text-gray-500 hover:text-red-500 hover:bg-red-50 dark:hover:bg-red-900/20 rounded-lg transition-colors disabled:opacity-50"
                  title="Delete provider"
                >
                  {deletingId === provider.id ? (
                    <div className="w-4 h-4 border-2 border-red-500 border-t-transparent rounded-full animate-spin" />
                  ) : (
                    <Trash2 size={18} />
                  )}
                </button>
              </div>
            </div>

            {/* Test result */}
            {testResult && testResult.id === provider.id && (
              <div
                className={`mt-3 p-3 rounded-lg flex items-start gap-2 ${
                  testResult.success
                    ? 'bg-green-50 dark:bg-green-900/20 text-green-700 dark:text-green-300'
                    : 'bg-red-50 dark:bg-red-900/20 text-red-700 dark:text-red-300'
                }`}
              >
                {testResult.success ? (
                  <CheckCircle size={18} className="flex-shrink-0 mt-0.5" />
                ) : (
                  <XCircle size={18} className="flex-shrink-0 mt-0.5" />
                )}
                <span className="text-sm">{testResult.message}</span>
              </div>
            )}

            {/* Stats row */}
            <div className="mt-3 pt-3 border-t border-gray-100 dark:border-gray-700 flex items-center gap-4 text-sm text-gray-500 dark:text-gray-400">
              <div className="flex items-center gap-1">
                <Server size={14} />
                <span>
                  {provider.app_count} app{provider.app_count !== 1 ? 's' : ''} using
                </span>
              </div>
              <div className="flex items-center gap-1">
                {provider.kill_switch ? (
                  <span className="text-green-500 flex items-center gap-1">
                    <ShieldCheck size={14} />
                    Kill switch ON
                  </span>
                ) : (
                  <span className="text-yellow-500 flex items-center gap-1">
                    <ShieldX size={14} />
                    Kill switch OFF
                  </span>
                )}
              </div>
              <div>
                {provider.enabled ? (
                  <span className="text-green-500">Enabled</span>
                ) : (
                  <span className="text-gray-400">Disabled</span>
                )}
              </div>
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}
