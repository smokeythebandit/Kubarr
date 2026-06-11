import React, { useState, useEffect } from 'react';
import { Check, X, Loader2, RefreshCw, Play } from 'lucide-react';
import { setupApi } from '../../api/setup';
import { useBootstrapWs } from '../../hooks/useBootstrapWs';
import { getBootstrapIcon } from './BootstrapIcons';

interface BootstrapStepProps {
  onComplete: () => void;
  onBack?: () => void;
}

const getStatusBgColor = (status: string) => {
  switch (status) {
    case 'pending':
      return 'bg-gray-100 dark:bg-gray-800';
    case 'installing':
      return 'bg-blue-50 dark:bg-blue-900/20';
    case 'healthy':
      return 'bg-green-50 dark:bg-green-900/20';
    case 'failed':
      return 'bg-red-50 dark:bg-red-900/20';
    default:
      return 'bg-gray-100 dark:bg-gray-800';
  }
};

const ComponentStatusIcon: React.FC<{ status: string }> = ({ status }) => {
  switch (status) {
    case 'pending':
      return <div className="w-6 h-6 rounded-full border-2 border-gray-300 dark:border-gray-600" />;
    case 'installing':
      return <Loader2 className="w-6 h-6 text-blue-500 animate-spin" />;
    case 'healthy':
      return (
        <div className="w-6 h-6 rounded-full bg-green-500 flex items-center justify-center">
          <Check className="w-4 h-4 text-white" />
        </div>
      );
    case 'failed':
      return (
        <div className="w-6 h-6 rounded-full bg-red-500 flex items-center justify-center">
          <X className="w-4 h-4 text-white" />
        </div>
      );
    default:
      return <div className="w-6 h-6 rounded-full border-2 border-gray-300 dark:border-gray-600" />;
  }
};

const BootstrapStep: React.FC<BootstrapStepProps> = ({ onComplete }) => {
  const [isStarting, setIsStarting] = useState(false);
  const [retrying, setRetrying] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const {
    components,
    isComplete,
    isStarted,
    isConnected,
  } = useBootstrapWs();

  // Auto-advance when bootstrap is complete
  useEffect(() => {
    if (isComplete) {
      // Small delay to show the success state
      const timer = setTimeout(() => {
        onComplete();
      }, 1500);
      return () => clearTimeout(timer);
    }
  }, [isComplete, onComplete]);

  const handleStartBootstrap = async () => {
    setIsStarting(true);
    setError(null);
    try {
      await setupApi.startBootstrap();
    } catch (err: any) {
      setError(err.response?.data?.detail || 'Failed to start bootstrap');
    } finally {
      setIsStarting(false);
    }
  };

  const handleRetry = async (component: string) => {
    setRetrying(component);
    setError(null);
    try {
      await setupApi.retryBootstrapComponent(component);
    } catch (err: any) {
      setError(err.response?.data?.detail || `Failed to retry ${component}`);
    } finally {
      setRetrying(null);
    }
  };

  const hasFailedComponents = components.some((c) => c.status === 'failed');
  const allHealthy = components.length > 0 && components.every((c) => c.status === 'healthy');
  const isInstalling = components.some((c) => c.status === 'installing');

  // Calculate overall progress
  const healthyCount = components.filter((c) => c.status === 'healthy').length;
  const progressPercent = components.length > 0 ? (healthyCount / components.length) * 100 : 0;

  return (
    <div className="space-y-6">
      <div>
        <h2 className="text-xl font-semibold text-gray-900 dark:text-white mb-2">
          System Setup
        </h2>
        <p className="text-gray-500 dark:text-gray-400 text-sm">
          Installing required system components for database, monitoring, and logging.
        </p>
      </div>

      {error && (
        <div className="rounded-md bg-red-900/50 border border-red-700 p-4">
          <div className="text-sm text-red-200">{error}</div>
        </div>
      )}

      {/* Progress bar */}
      {isStarted && (
        <div className="space-y-2">
          <div className="flex justify-between text-sm text-gray-500 dark:text-gray-400">
            <span>Progress</span>
            <span>{healthyCount} of {components.length} components</span>
          </div>
          <div className="h-2 bg-gray-200 dark:bg-gray-700 rounded-full overflow-hidden">
            <div
              className={`h-full transition-all duration-500 ${
                hasFailedComponents ? 'bg-red-500' : 'bg-blue-500'
              }`}
              style={{ width: `${progressPercent}%` }}
            />
          </div>
        </div>
      )}

      {/* Component list */}
      <div className="space-y-3">
        {components.map((component) => (
          <div
            key={component.component}
            className={`rounded-lg border border-gray-200 dark:border-gray-700 p-4 ${getStatusBgColor(component.status)}`}
          >
            <div className="flex items-center justify-between">
              <div className="flex items-center space-x-4">
                {/* App Icon */}
                <div className="flex-shrink-0">
                  {getBootstrapIcon(component.component, 'w-10 h-10')}
                </div>
                {/* Component Info */}
                <div className="flex-1 min-w-0">
                  <h3 className="font-medium text-gray-900 dark:text-white">
                    {component.display_name}
                  </h3>
                  <p className="text-sm text-gray-500 dark:text-gray-400 truncate">
                    {component.message || getComponentDescription(component.component)}
                  </p>
                </div>
              </div>

              {/* Right side: Status icon and retry button */}
              <div className="flex items-center space-x-3 ml-4">
                {component.status === 'failed' && (
                  <button
                    type="button"
                    onClick={() => handleRetry(component.component)}
                    disabled={retrying === component.component}
                    className="flex items-center space-x-2 px-3 py-1.5 text-sm bg-red-600 text-white rounded-md hover:bg-red-700 disabled:opacity-50"
                  >
                    {retrying === component.component ? (
                      <Loader2 className="w-4 h-4 animate-spin" />
                    ) : (
                      <RefreshCw className="w-4 h-4" />
                    )}
                    <span>Retry</span>
                  </button>
                )}
                <ComponentStatusIcon status={component.status} />
              </div>
            </div>

            {component.status === 'failed' && component.error && (
              <div className="mt-3 p-3 bg-red-900/30 rounded text-sm text-red-300 font-mono">
                {component.error}
              </div>
            )}
          </div>
        ))}
      </div>

      {/* Connection status indicator */}
      {isStarted && !isComplete && (
        <div className="flex items-center space-x-2 text-xs text-gray-500 dark:text-gray-400">
          <div
            className={`w-2 h-2 rounded-full ${
              isConnected ? 'bg-green-500' : 'bg-yellow-500'
            }`}
          />
          <span>
            {isConnected ? 'Connected (real-time updates)' : 'Polling for updates...'}
          </span>
        </div>
      )}

      {/* Action buttons */}
      <div className="flex justify-end space-x-4">
        {!isStarted ? (
          <button
            type="button"
            onClick={handleStartBootstrap}
            disabled={isStarting || components.length === 0}
            className="flex items-center space-x-2 px-6 py-2 bg-blue-600 text-white font-medium rounded-md hover:bg-blue-700 focus:outline-none focus:ring-2 focus:ring-blue-500 disabled:opacity-50"
          >
            {isStarting ? (
              <Loader2 className="w-5 h-5 animate-spin" />
            ) : (
              <Play className="w-5 h-5" />
            )}
            <span>{isStarting ? 'Starting...' : 'Start Setup'}</span>
          </button>
        ) : isComplete || allHealthy ? (
          <button
            type="button"
            onClick={onComplete}
            className="flex items-center space-x-2 px-6 py-2 bg-green-600 text-white font-medium rounded-md hover:bg-green-700 focus:outline-none focus:ring-2 focus:ring-green-500"
          >
            <Check className="w-5 h-5" />
            <span>Continue</span>
          </button>
        ) : isInstalling ? (
          <button
            type="button"
            disabled
            className="px-6 py-2 bg-gray-600 text-white font-medium rounded-md opacity-50 cursor-not-allowed"
          >
            <span className="flex items-center space-x-2">
              <Loader2 className="w-5 h-5 animate-spin" />
              <span>Installing...</span>
            </span>
          </button>
        ) : (
          <button
            type="button"
            onClick={handleStartBootstrap}
            disabled={isStarting}
            className="flex items-center space-x-2 px-6 py-2 bg-blue-600 text-white font-medium rounded-md hover:bg-blue-700 focus:outline-none focus:ring-2 focus:ring-blue-500 disabled:opacity-50"
          >
            {isStarting ? (
              <Loader2 className="w-5 h-5 animate-spin" />
            ) : (
              <RefreshCw className="w-5 h-5" />
            )}
            <span>{isStarting ? 'Resuming...' : 'Resume Setup'}</span>
          </button>
        )}
      </div>
    </div>
  );
};

function getComponentDescription(component: string): string {
  switch (component) {
    case 'postgresql':
      return 'Database for application data';
    case 'victoriametrics':
      return 'Time-series database for metrics storage';
    case 'victorialogs':
      return 'Log aggregation and storage';
    case 'fluent-bit':
      return 'Lightweight log collector and forwarder';
    default:
      return 'System component';
  }
}

export default BootstrapStep;
