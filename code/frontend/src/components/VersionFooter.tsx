import { useState, useEffect } from 'react';

interface BackendVersion {
  version: string;
  channel: string;
  commit_hash: string;
  build_time: string;
}

type Channel = 'dev' | 'release' | 'stable';

export function VersionFooter() {
  const [backendVersion, setBackendVersion] = useState<BackendVersion | null>(null);

  const frontendVersion = __VERSION__;
  const frontendChannel = __CHANNEL__ as Channel;
  const frontendCommit = __COMMIT_HASH__;
  const frontendBuildTime = __BUILD_TIME__;

  // Channel badge colors
  const getChannelColor = (channel: Channel) => {
    switch (channel) {
      case 'stable':
        return 'bg-green-100 text-green-800 dark:bg-green-900 dark:text-green-200';
      case 'release':
        return 'bg-yellow-100 text-yellow-800 dark:bg-yellow-900 dark:text-yellow-200';
      case 'dev':
      default:
        return 'bg-blue-100 text-blue-800 dark:bg-blue-900 dark:text-blue-200';
    }
  };

  useEffect(() => {
    fetch('/api/system/version')
      .then(res => res.json())
      .then(data => setBackendVersion(data))
      .catch(() => setBackendVersion({
        version: 'error',
        channel: 'dev',
        commit_hash: 'error',
        build_time: ''
      }));
  }, []);

  return (
    <footer className="fixed bottom-0 left-0 right-0 bg-white dark:bg-gray-900 border-t border-gray-200 dark:border-gray-700 px-4 py-2 text-xs text-gray-500">
      <div className="flex justify-center items-center gap-6">
        <span className="flex items-center gap-2">
          <span className={`px-2 py-0.5 rounded text-xs font-medium ${getChannelColor(frontendChannel)}`}>
            {frontendChannel}
          </span>
          <span>v{frontendVersion}</span>
          <code className="text-gray-600 dark:text-gray-400">{frontendCommit.substring(0, 7)}</code>
          <span className="text-gray-400 dark:text-gray-600">({new Date(frontendBuildTime).toLocaleDateString()})</span>
        </span>

        <span className="text-gray-300 dark:text-gray-700">|</span>

        <span className="flex items-center gap-2">
          {backendVersion && (
            <>
              <span className={`px-2 py-0.5 rounded text-xs font-medium ${getChannelColor(backendVersion.channel as Channel)}`}>
                {backendVersion.channel}
              </span>
              <span>v{backendVersion.version}</span>
              <code className="text-gray-600 dark:text-gray-400">{backendVersion.commit_hash.substring(0, 7)}</code>
              {backendVersion.build_time && (
                <span className="text-gray-400 dark:text-gray-600">({new Date(backendVersion.build_time).toLocaleDateString()})</span>
              )}
            </>
          )}
          {!backendVersion && <span>Loading backend version...</span>}
        </span>
      </div>
    </footer>
  );
}
