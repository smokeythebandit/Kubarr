import React, { useEffect, useState } from 'react';
import { AlertCircle, Loader2 } from 'lucide-react';
import { setupApi, ServerConfigResponse } from '../../api/setup';

interface ServerStepProps {
  onComplete: (config: { name: string }) => void;
  onBack: () => void;
  initialConfig?: ServerConfigResponse | null;
}

const ServerStep: React.FC<ServerStepProps> = ({ onComplete, onBack, initialConfig }) => {
  const [serverName, setServerName] = useState(initialConfig?.name || 'Kubarr');
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    const loadConfig = async () => {
      try {
        const config = await setupApi.getServerConfig();
        if (config) {
          setServerName(config.name);
        }
      } catch {
        // Config does not exist yet.
      }
    };
    if (!initialConfig) {
      loadConfig();
    }
  }, [initialConfig]);

  const handleSubmit = async () => {
    setError(null);

    if (!serverName.trim()) {
      setError('Server name is required');
      return;
    }

    setLoading(true);

    try {
      const config = await setupApi.configureServer({
        name: serverName.trim(),
      });

      onComplete({ name: config.name });
    } catch (err: any) {
      setError(err.response?.data?.detail || 'Failed to save server configuration');
    } finally {
      setLoading(false);
    }
  };

  return (
    <div className="space-y-6">
      <div>
        <h2 className="text-xl font-semibold text-gray-900 dark:text-white mb-2">
          Server Name
        </h2>
        <p className="text-gray-500 dark:text-gray-400 text-sm">
          Set the display name for this Kubarr instance.
        </p>
      </div>

      {error && (
        <div className="rounded-md bg-red-900/50 border border-red-700 p-4">
          <div className="flex items-center space-x-2">
            <AlertCircle className="w-5 h-5 text-red-400" />
            <span className="text-sm text-red-200">{error}</span>
          </div>
        </div>
      )}

      <div>
        <label htmlFor="serverName" className="block text-sm font-medium text-gray-700 dark:text-gray-300">
          Server Name
        </label>
        <input
          type="text"
          id="serverName"
          value={serverName}
          onChange={(e) => setServerName(e.target.value)}
          className="mt-1 block w-full px-3 py-2 border border-gray-300 dark:border-gray-700 bg-white dark:bg-gray-900 text-gray-900 dark:text-white rounded-md focus:outline-none focus:ring-blue-500 focus:border-blue-500 sm:text-sm"
          placeholder="My Kubarr Server"
        />
      </div>

      <div className="flex justify-between">
        <button
          type="button"
          onClick={onBack}
          disabled={loading}
          className="px-6 py-2 border border-gray-600 text-gray-300 font-medium rounded-md hover:bg-gray-700 focus:outline-none focus:ring-2 focus:ring-gray-500 disabled:opacity-50"
        >
          Back
        </button>
        <button
          type="button"
          onClick={handleSubmit}
          disabled={loading}
          className="px-6 py-2 bg-blue-600 text-white font-medium rounded-md hover:bg-blue-700 focus:outline-none focus:ring-2 focus:ring-blue-500 disabled:opacity-50"
        >
          {loading ? (
            <span className="flex items-center space-x-2">
              <Loader2 className="w-4 h-4 animate-spin" />
              <span>Saving...</span>
            </span>
          ) : (
            'Next'
          )}
        </button>
      </div>
    </div>
  );
};

export default ServerStep;
