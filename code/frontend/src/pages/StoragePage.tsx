import { useState } from 'react'
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { Folder, File, ChevronRight, Home, Plus, Trash2, RefreshCw, Download, ArrowUp } from 'lucide-react'
import { storageApi, formatBytes, formatDate } from '../api/storage'
import type { FileInfo } from '../api/storage'
import { useAuth } from '../contexts/AuthContext'

export default function StoragePage() {
  const [currentPath, setCurrentPath] = useState('')
  const [showNewFolderModal, setShowNewFolderModal] = useState(false)
  const [newFolderName, setNewFolderName] = useState('')
  const [selectedItem, setSelectedItem] = useState<FileInfo | null>(null)
  const [showDeleteConfirm, setShowDeleteConfirm] = useState(false)

  const { isAdmin } = useAuth()
  const queryClient = useQueryClient()

  // Fetch storage stats
  const { data: stats, isLoading: statsLoading } = useQuery({
    queryKey: ['storage', 'stats'],
    queryFn: storageApi.getStats,
    refetchInterval: 30000, // Refresh every 30 seconds
  })

  // Fetch directory listing
  const {
    data: listing,
    isLoading: listingLoading,
    error: listingError,
    refetch: refetchListing,
  } = useQuery({
    queryKey: ['storage', 'browse', currentPath],
    queryFn: () => storageApi.browse(currentPath),
  })

  // Create directory mutation
  const createDirMutation = useMutation({
    mutationFn: (path: string) => storageApi.createDirectory(path),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['storage', 'browse'] })
      setShowNewFolderModal(false)
      setNewFolderName('')
    },
  })

  // Delete mutation
  const deleteMutation = useMutation({
    mutationFn: (path: string) => storageApi.deletePath(path),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['storage', 'browse'] })
      queryClient.invalidateQueries({ queryKey: ['storage', 'stats'] })
      setShowDeleteConfirm(false)
      setSelectedItem(null)
    },
  })

  const handleNavigate = (path: string) => {
    setCurrentPath(path)
    setSelectedItem(null)
  }

  const handleItemClick = (item: FileInfo) => {
    if (item.type === 'directory') {
      handleNavigate(item.path)
    } else {
      setSelectedItem(item)
    }
  }

  const handleCreateFolder = () => {
    if (!newFolderName.trim()) return
    const fullPath = currentPath ? `${currentPath}/${newFolderName}` : newFolderName
    createDirMutation.mutate(fullPath)
  }

  const handleDelete = () => {
    if (!selectedItem) return
    deleteMutation.mutate(selectedItem.path)
  }

  const handleDownload = () => {
    if (!selectedItem || selectedItem.type === 'directory') return
    const url = storageApi.getDownloadUrl(selectedItem.path)
    window.open(url, '_blank')
  }

  // Build breadcrumb path segments
  const pathSegments = currentPath ? currentPath.split('/').filter(Boolean) : []
  const breadcrumbs = [
    { name: 'Root', path: '' },
    ...pathSegments.map((segment, index) => ({
      name: segment,
      path: pathSegments.slice(0, index + 1).join('/'),
    })),
  ]

  const isLoading = statsLoading || listingLoading

  if (listingError) {
    return (
      <div className="space-y-6">
        <h1 className="text-2xl font-bold text-gray-900 dark:text-white">Storage</h1>
        <div className="bg-red-100 dark:bg-red-900/50 border border-red-300 dark:border-red-500 rounded-lg p-6">
          <h3 className="text-lg font-semibold text-red-600 dark:text-red-400 mb-2">Storage Not Available</h3>
          <p className="text-gray-600 dark:text-gray-300">
            Shared storage is not configured or not accessible. To enable file browsing:
          </p>
          <ol className="list-decimal list-inside mt-3 text-gray-500 dark:text-gray-400 space-y-1">
            <li>Deploy the shared-storage Helm chart</li>
            <li>Update kubarr chart with sharedStorage.enabled=true</li>
            <li>Ensure the storage PVC is bound and accessible</li>
          </ol>
        </div>
      </div>
    )
  }

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <h1 className="text-2xl font-bold text-gray-900 dark:text-white">Storage</h1>
        <button
          onClick={() => refetchListing()}
          className="flex items-center gap-2 px-3 py-2 text-sm text-gray-500 dark:text-gray-400 hover:text-gray-900 dark:hover:text-white hover:bg-gray-200 dark:hover:bg-gray-700 rounded transition-colors"
        >
          <RefreshCw size={16} />
          Refresh
        </button>
      </div>

      {/* Storage Stats */}
      {stats && (
        <div className="bg-white dark:bg-gray-800 rounded-lg p-6 border border-gray-200 dark:border-gray-700">
          <h2 className="text-lg font-semibold text-gray-900 dark:text-white mb-4">Storage Usage</h2>
          <div className="space-y-3">
            <div className="flex justify-between text-sm">
              <span className="text-gray-500 dark:text-gray-400">Used: {formatBytes(stats.used_bytes)}</span>
              <span className="text-gray-500 dark:text-gray-400">Free: {formatBytes(stats.free_bytes)}</span>
              <span className="text-gray-500 dark:text-gray-400">Total: {formatBytes(stats.total_bytes)}</span>
            </div>
            <div className="h-4 bg-gray-200 dark:bg-gray-700 rounded-full overflow-hidden">
              <div
                className={`h-full rounded-full transition-all ${
                  stats.usage_percent > 90
                    ? 'bg-red-500'
                    : stats.usage_percent > 70
                    ? 'bg-yellow-500'
                    : 'bg-blue-500'
                }`}
                style={{ width: `${Math.min(stats.usage_percent, 100)}%` }}
              />
            </div>
            <div className="text-center text-sm text-gray-500 dark:text-gray-400">{stats.usage_percent}% used</div>
          </div>
        </div>
      )}

      {/* Breadcrumb Navigation */}
      <div className="bg-white dark:bg-gray-800 rounded-lg p-4 border border-gray-200 dark:border-gray-700">
        <div className="flex items-center gap-2 flex-wrap">
          {breadcrumbs.map((crumb, index) => (
            <div key={crumb.path} className="flex items-center">
              {index > 0 && <ChevronRight size={16} className="text-gray-400 dark:text-gray-500 mx-1" />}
              <button
                onClick={() => handleNavigate(crumb.path)}
                className={`flex items-center gap-1 px-2 py-1 rounded hover:bg-gray-100 dark:hover:bg-gray-700 transition-colors ${
                  index === breadcrumbs.length - 1 ? 'text-gray-900 dark:text-white font-medium' : 'text-gray-500 dark:text-gray-400'
                }`}
              >
                {index === 0 && <Home size={16} />}
                {crumb.name}
              </button>
            </div>
          ))}
        </div>
      </div>

      {/* Actions Bar */}
      <div className="flex gap-3">
        {isAdmin && (
          <button
            onClick={() => setShowNewFolderModal(true)}
            className="flex items-center gap-2 px-4 py-2 bg-blue-600 hover:bg-blue-700 rounded transition-colors"
          >
            <Plus size={16} />
            New Folder
          </button>
        )}
        {selectedItem && selectedItem.type === 'file' && (
          <button
            onClick={handleDownload}
            className="flex items-center gap-2 px-4 py-2 bg-green-600 hover:bg-green-700 rounded transition-colors"
          >
            <Download size={16} />
            Download
          </button>
        )}
        {isAdmin && selectedItem && (
          <button
            onClick={() => setShowDeleteConfirm(true)}
            className="flex items-center gap-2 px-4 py-2 bg-red-600 hover:bg-red-700 rounded transition-colors"
          >
            <Trash2 size={16} />
            Delete Selected
          </button>
        )}
      </div>

      {/* File Listing */}
      <div className="bg-white dark:bg-gray-800 rounded-lg overflow-hidden border border-gray-200 dark:border-gray-700">
        {isLoading ? (
          <div className="flex items-center justify-center h-64">
            <div className="text-gray-500 dark:text-gray-400">Loading...</div>
          </div>
        ) : listing?.items.length === 0 ? (
          <div className="flex flex-col items-center justify-center h-64 text-gray-500 dark:text-gray-400">
            <Folder size={48} className="mb-4 opacity-50" />
            <p>This folder is empty</p>
          </div>
        ) : (
          <table className="w-full">
            <thead className="bg-gray-100 dark:bg-gray-700">
              <tr>
                <th className="text-left px-4 py-3 text-sm font-medium text-gray-600 dark:text-gray-300">Name</th>
                <th className="text-left px-4 py-3 text-sm font-medium text-gray-600 dark:text-gray-300">Size</th>
                <th className="text-left px-4 py-3 text-sm font-medium text-gray-600 dark:text-gray-300">Modified</th>
                <th className="text-left px-4 py-3 text-sm font-medium text-gray-600 dark:text-gray-300">Permissions</th>
              </tr>
            </thead>
            <tbody className="divide-y divide-gray-200 dark:divide-gray-700">
              {/* Parent directory row */}
              {listing?.parent !== undefined && listing?.parent !== null && (
                <tr
                  onClick={() => handleNavigate(listing.parent!)}
                  className="cursor-pointer hover:bg-gray-100 dark:hover:bg-gray-700 transition-colors"
                >
                  <td className="px-4 py-3">
                    <div className="flex items-center gap-3">
                      <ArrowUp size={20} className="text-blue-500 dark:text-blue-400" />
                      <span className="text-gray-900 dark:text-white">..</span>
                    </div>
                  </td>
                  <td className="px-4 py-3 text-gray-500 dark:text-gray-400">-</td>
                  <td className="px-4 py-3 text-gray-500 dark:text-gray-400">-</td>
                  <td className="px-4 py-3 text-gray-500 dark:text-gray-400">-</td>
                </tr>
              )}
              {listing?.items.map((item) => (
                <tr
                  key={item.path}
                  onClick={() => handleItemClick(item)}
                  className={`cursor-pointer hover:bg-gray-100 dark:hover:bg-gray-700 transition-colors ${
                    selectedItem?.path === item.path ? 'bg-blue-100 dark:bg-blue-900/30' : ''
                  }`}
                >
                  <td className="px-4 py-3">
                    <div className="flex items-center gap-3">
                      {item.type === 'directory' ? (
                        <Folder size={20} className="text-yellow-500 dark:text-yellow-400" />
                      ) : (
                        <File size={20} className="text-gray-400" />
                      )}
                      <span className="text-gray-900 dark:text-white">{item.name}</span>
                    </div>
                  </td>
                  <td className="px-4 py-3 text-gray-500 dark:text-gray-400">
                    {item.type === 'directory' ? '-' : formatBytes(item.size)}
                  </td>
                  <td className="px-4 py-3 text-gray-500 dark:text-gray-400">{formatDate(item.modified)}</td>
                  <td className="px-4 py-3 text-gray-500 dark:text-gray-400 font-mono">{item.permissions}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </div>

      {/* Item count */}
      {listing && (
        <div className="text-sm text-gray-500 dark:text-gray-400">
          {listing.total_items} item{listing.total_items !== 1 ? 's' : ''}
        </div>
      )}

      {/* New Folder Modal */}
      {showNewFolderModal && (
        <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
          <div className="bg-white dark:bg-gray-800 rounded-lg p-6 w-full max-w-md border border-gray-200 dark:border-gray-700">
            <h3 className="text-lg font-semibold text-gray-900 dark:text-white mb-4">Create New Folder</h3>
            <input
              type="text"
              value={newFolderName}
              onChange={(e) => setNewFolderName(e.target.value)}
              placeholder="Folder name"
              className="w-full px-4 py-2 bg-gray-100 dark:bg-gray-700 border border-gray-300 dark:border-gray-600 rounded text-gray-900 dark:text-white placeholder-gray-500 dark:placeholder-gray-400 focus:outline-none focus:border-blue-500"
              autoFocus
              onKeyDown={(e) => {
                if (e.key === 'Enter') handleCreateFolder()
                if (e.key === 'Escape') setShowNewFolderModal(false)
              }}
            />
            {createDirMutation.error && (
              <p className="mt-2 text-sm text-red-500 dark:text-red-400">
                {(createDirMutation.error as Error).message || 'Failed to create folder'}
              </p>
            )}
            <div className="flex justify-end gap-3 mt-4">
              <button
                onClick={() => {
                  setShowNewFolderModal(false)
                  setNewFolderName('')
                }}
                className="px-4 py-2 text-gray-500 dark:text-gray-400 hover:text-gray-900 dark:hover:text-white transition-colors"
              >
                Cancel
              </button>
              <button
                onClick={handleCreateFolder}
                disabled={!newFolderName.trim() || createDirMutation.isPending}
                className="px-4 py-2 bg-blue-600 hover:bg-blue-700 disabled:opacity-50 disabled:cursor-not-allowed rounded transition-colors"
              >
                {createDirMutation.isPending ? 'Creating...' : 'Create'}
              </button>
            </div>
          </div>
        </div>
      )}

      {/* Delete Confirmation Modal */}
      {showDeleteConfirm && selectedItem && (
        <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
          <div className="bg-white dark:bg-gray-800 rounded-lg p-6 w-full max-w-md border border-gray-200 dark:border-gray-700">
            <h3 className="text-lg font-semibold text-gray-900 dark:text-white mb-4">Delete {selectedItem.type === 'directory' ? 'Folder' : 'File'}</h3>
            <p className="text-gray-600 dark:text-gray-300 mb-4">
              Are you sure you want to delete <span className="font-mono text-gray-900 dark:text-white">{selectedItem.name}</span>?
              {selectedItem.type === 'directory' && (
                <span className="block mt-2 text-yellow-600 dark:text-yellow-400 text-sm">
                  Note: Only empty directories can be deleted.
                </span>
              )}
            </p>
            {deleteMutation.error && (
              <p className="mb-4 text-sm text-red-500 dark:text-red-400">
                {(deleteMutation.error as Error).message || 'Failed to delete'}
              </p>
            )}
            <div className="flex justify-end gap-3">
              <button
                onClick={() => {
                  setShowDeleteConfirm(false)
                  setSelectedItem(null)
                }}
                className="px-4 py-2 text-gray-500 dark:text-gray-400 hover:text-gray-900 dark:hover:text-white transition-colors"
              >
                Cancel
              </button>
              <button
                onClick={handleDelete}
                disabled={deleteMutation.isPending}
                className="px-4 py-2 bg-red-600 hover:bg-red-700 disabled:opacity-50 disabled:cursor-not-allowed rounded transition-colors"
              >
                {deleteMutation.isPending ? 'Deleting...' : 'Delete'}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  )
}
