import React, { useState, useEffect, useCallback } from 'react';
import { Folder, ChevronRight, Home, Loader2, AlertCircle } from 'lucide-react';
import { setupApi } from '../../api/setup';
import type { SetupDirectoryEntry } from '../../api/setup';

interface DirectoryPickerProps {
  isOpen: boolean;
  initialPath: string;
  onSelect: (path: string) => void;
  onClose: () => void;
}

const DirectoryPicker: React.FC<DirectoryPickerProps> = ({
  isOpen,
  initialPath,
  onSelect,
  onClose,
}) => {
  const [currentPath, setCurrentPath] = useState('/');
  const [directories, setDirectories] = useState<SetupDirectoryEntry[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const fetchDirectory = useCallback(async (path: string) => {
    setLoading(true);
    setError(null);
    try {
      const result = await setupApi.browsePath(path);
      setCurrentPath(result.path);
      setDirectories(result.directories);
    } catch (err: any) {
      // If the requested path fails, fall back to root
      if (path !== '/') {
        try {
          const result = await setupApi.browsePath('/');
          setCurrentPath(result.path);
          setDirectories(result.directories);
        } catch {
          setError('Failed to browse filesystem');
        }
      } else {
        setError(err.response?.data?.detail || 'Failed to browse filesystem');
      }
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    if (isOpen) {
      const startPath = initialPath.trim() || '/';
      fetchDirectory(startPath);
    }
  }, [isOpen, initialPath, fetchDirectory]);

  if (!isOpen) return null;

  // Build breadcrumb segments from currentPath
  const pathSegments = currentPath === '/'
    ? []
    : currentPath.split('/').filter(Boolean);

  const breadcrumbs = [
    { name: '/', path: '/' },
    ...pathSegments.map((segment, index) => ({
      name: segment,
      path: '/' + pathSegments.slice(0, index + 1).join('/'),
    })),
  ];

  const parentPath = currentPath === '/'
    ? null
    : '/' + pathSegments.slice(0, -1).join('/') || '/';

  const handleSelect = () => {
    onSelect(currentPath);
    onClose();
  };

  return (
    <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
      <div className="bg-white dark:bg-gray-800 rounded-lg w-full max-w-lg border border-gray-200 dark:border-gray-700 flex flex-col max-h-[80vh]">
        {/* Header */}
        <div className="p-4 border-b border-gray-200 dark:border-gray-700">
          <h3 className="text-lg font-semibold text-gray-900 dark:text-white">
            Select Storage Directory
          </h3>
          <p className="text-sm text-gray-500 dark:text-gray-400 mt-1 font-mono truncate">
            {currentPath}
          </p>
        </div>

        {/* Breadcrumbs */}
        <div className="px-4 py-2 border-b border-gray-200 dark:border-gray-700 flex items-center gap-1 flex-wrap overflow-x-auto">
          {breadcrumbs.map((crumb, index) => (
            <div key={crumb.path} className="flex items-center">
              {index > 0 && (
                <ChevronRight size={14} className="text-gray-400 dark:text-gray-500 mx-0.5 flex-shrink-0" />
              )}
              <button
                onClick={() => fetchDirectory(crumb.path)}
                className={`flex items-center gap-1 px-1.5 py-0.5 rounded text-sm hover:bg-gray-100 dark:hover:bg-gray-700 transition-colors ${
                  index === breadcrumbs.length - 1
                    ? 'text-gray-900 dark:text-white font-medium'
                    : 'text-gray-500 dark:text-gray-400'
                }`}
              >
                {index === 0 && <Home size={14} />}
                {crumb.name}
              </button>
            </div>
          ))}
        </div>

        {/* Directory listing */}
        <div className="flex-1 overflow-y-auto min-h-0">
          {loading ? (
            <div className="flex items-center justify-center h-48">
              <Loader2 className="w-6 h-6 animate-spin text-gray-400" />
            </div>
          ) : error ? (
            <div className="flex flex-col items-center justify-center h-48 px-4">
              <AlertCircle className="w-8 h-8 text-red-400 mb-2" />
              <p className="text-sm text-red-400 text-center">{error}</p>
            </div>
          ) : (
            <div className="divide-y divide-gray-200 dark:divide-gray-700">
              {/* Parent directory entry */}
              {parentPath !== null && (
                <button
                  onClick={() => fetchDirectory(parentPath)}
                  className="w-full flex items-center gap-3 px-4 py-2.5 hover:bg-gray-100 dark:hover:bg-gray-700 transition-colors text-left"
                >
                  <Folder size={18} className="text-gray-400 flex-shrink-0" />
                  <span className="text-gray-500 dark:text-gray-400 text-sm">..</span>
                </button>
              )}
              {directories.length === 0 && parentPath === null ? (
                <div className="flex flex-col items-center justify-center h-48 text-gray-500 dark:text-gray-400">
                  <Folder size={36} className="mb-3 opacity-50" />
                  <p className="text-sm">No subdirectories</p>
                </div>
              ) : directories.length === 0 ? (
                <div className="flex flex-col items-center justify-center h-32 text-gray-500 dark:text-gray-400">
                  <p className="text-sm">No subdirectories</p>
                </div>
              ) : (
                directories.map((dir) => (
                  <button
                    key={dir.path}
                    onClick={() => fetchDirectory(dir.path)}
                    className="w-full flex items-center gap-3 px-4 py-2.5 hover:bg-gray-100 dark:hover:bg-gray-700 transition-colors text-left"
                  >
                    <Folder size={18} className="text-yellow-500 dark:text-yellow-400 flex-shrink-0" />
                    <span className="text-gray-900 dark:text-white text-sm truncate">{dir.name}</span>
                  </button>
                ))
              )}
            </div>
          )}
        </div>

        {/* Footer */}
        <div className="p-4 border-t border-gray-200 dark:border-gray-700 flex justify-end gap-3">
          <button
            onClick={onClose}
            className="px-4 py-2 text-sm text-gray-500 dark:text-gray-400 hover:text-gray-900 dark:hover:text-white transition-colors"
          >
            Cancel
          </button>
          <button
            onClick={handleSelect}
            disabled={loading}
            className="px-4 py-2 text-sm bg-blue-600 text-white rounded-md hover:bg-blue-700 focus:outline-none focus:ring-2 focus:ring-blue-500 disabled:opacity-50"
          >
            Select this directory
          </button>
        </div>
      </div>
    </div>
  );
};

export default DirectoryPicker;
