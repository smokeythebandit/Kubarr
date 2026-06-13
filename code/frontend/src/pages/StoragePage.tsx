import { useEffect, useState } from 'react'
import type { ReactNode } from 'react'
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { ChevronRight, Grid3X3, Home, List, Plus, RefreshCw } from 'lucide-react'
import { storageApi } from '../api/storage'
import type { FileInfo } from '../api/storage'
import { StorageBrowser } from '../components/storage/StorageBrowser'
import type { BrowserViewMode } from '../components/storage/StorageBrowser'
import { StorageEditorPane } from '../components/storage/StorageEditorPane'
import { StorageTabs } from '../components/storage/StorageTabs'
import { WinDirStatView } from '../components/storage/WinDirStatView'
import { isEditableFile } from '../components/storage/storageUtils'
import { useAuth } from '../contexts/AuthContext'

type StorageTab = 'browser' | 'windirstat'

export default function StoragePage() {
  const [activeTab, setActiveTab] = useState<StorageTab>('browser')
  const [browserViewMode, setBrowserViewMode] = useState<BrowserViewMode>('table')
  const [currentPath, setCurrentPath] = useState('')
  const [showNewFolderModal, setShowNewFolderModal] = useState(false)
  const [newFolderName, setNewFolderName] = useState('')
  const [selectedItem, setSelectedItem] = useState<FileInfo | null>(null)
  const [showDeleteConfirm, setShowDeleteConfirm] = useState(false)
  const [showRenameModal, setShowRenameModal] = useState(false)
  const [renameValue, setRenameValue] = useState('')
  const [editorItem, setEditorItem] = useState<FileInfo | null>(null)
  const [editorContent, setEditorContent] = useState('')

  const { isAdmin } = useAuth()
  const queryClient = useQueryClient()

  const {
    data: listing,
    isLoading: listingLoading,
    error: listingError,
    refetch: refetchListing,
  } = useQuery({
    queryKey: ['storage', 'browse', currentPath],
    queryFn: () => storageApi.browse(currentPath),
  })

  const {
    data: usage,
    isLoading: usageLoading,
    error: usageError,
    refetch: refetchUsage,
  } = useQuery({
    queryKey: ['storage', 'usage'],
    queryFn: storageApi.getUsage,
    enabled: activeTab === 'windirstat',
    staleTime: 30000,
  })

  const createDirMutation = useMutation({
    mutationFn: (path: string) => storageApi.createDirectory(path),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['storage', 'browse'] })
      queryClient.invalidateQueries({ queryKey: ['storage', 'usage'] })
      setShowNewFolderModal(false)
      setNewFolderName('')
    },
  })

  const deleteMutation = useMutation({
    mutationFn: (path: string) => storageApi.deletePath(path),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['storage', 'browse'] })
      queryClient.invalidateQueries({ queryKey: ['storage', 'usage'] })
      setShowDeleteConfirm(false)
      setSelectedItem(null)
      handleCloseEditor()
    },
  })

  const renameMutation = useMutation({
    mutationFn: () => storageApi.renamePath(selectedItem!.path, renameValue),
    onSuccess: (item) => {
      const wasEditingRenamedFile = editorItem?.path === selectedItem?.path
      queryClient.invalidateQueries({ queryKey: ['storage', 'browse'] })
      queryClient.invalidateQueries({ queryKey: ['storage', 'usage'] })
      setSelectedItem(item)
      setShowRenameModal(false)
      setRenameValue('')
      if (wasEditingRenamedFile) setEditorItem(item)
    },
  })

  const editorQuery = useQuery({
    queryKey: ['storage', 'text', editorItem?.path],
    queryFn: () => storageApi.readTextFile(editorItem!.path),
    enabled: !!editorItem,
  })

  const saveTextMutation = useMutation({
    mutationFn: () => storageApi.writeTextFile(editorItem!.path, editorContent),
    onSuccess: (file) => {
      setEditorContent(file.content)
      queryClient.invalidateQueries({ queryKey: ['storage', 'browse'] })
      queryClient.invalidateQueries({ queryKey: ['storage', 'usage'] })
    },
  })

  const textFile = editorQuery.data

  useEffect(() => {
    if (textFile && editorItem) setEditorContent(textFile.content)
  }, [textFile, editorItem])

  const handleNavigate = (path: string) => {
    setCurrentPath(path)
    setSelectedItem(null)
    handleCloseEditor()
  }

  const handleItemClick = (item: FileInfo) => {
    if (item.type === 'directory') {
      handleNavigate(item.path)
      return
    }

    setSelectedItem(item)
    if (isEditableFile(item)) {
      setEditorItem(item)
      setEditorContent('')
    } else {
      handleCloseEditor()
    }
  }

  const handleCloseEditor = () => {
    setEditorItem(null)
    setEditorContent('')
  }

  const handleCreateFolder = () => {
    if (!newFolderName.trim()) return
    const fullPath = currentPath ? `${currentPath}/${newFolderName}` : newFolderName
    createDirMutation.mutate(fullPath)
  }

  const handleDownloadItem = (item: FileInfo) => {
    if (item.type === 'directory') return
    window.open(storageApi.getDownloadUrl(item.path), '_blank')
  }

  const handleRequestDelete = (item: FileInfo) => {
    setSelectedItem(item)
    setShowDeleteConfirm(true)
  }

  const handleRequestRename = (item: FileInfo) => {
    setSelectedItem(item)
    setRenameValue(item.name)
    setShowRenameModal(true)
  }

  const handleDelete = () => {
    if (!selectedItem) return
    deleteMutation.mutate(selectedItem.path)
  }

  const openWinDirStatDirectory = (path: string) => {
    setCurrentPath(path)
    setActiveTab('browser')
  }

  const pathSegments = currentPath ? currentPath.split('/').filter(Boolean) : []
  const breadcrumbs = [
    { name: 'Root', path: '' },
    ...pathSegments.map((segment, index) => ({
      name: segment,
      path: pathSegments.slice(0, index + 1).join('/'),
    })),
  ]
  const folders = listing?.items.filter((item) => item.type === 'directory').length ?? 0
  const files = listing?.items.filter((item) => item.type === 'file').length ?? 0

  if (listingError) {
    return <StorageUnavailable />
  }

  return (
    <div className="max-w-full space-y-6 overflow-hidden">
      <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
        <div className="flex flex-col gap-3 sm:flex-row sm:items-center">
          <div>
            <h1 className="text-2xl font-black tracking-tight text-gray-950 dark:text-white">Storage</h1>
            <p className="mt-1 text-sm text-gray-500 dark:text-gray-400">NFS volume at <span className="font-mono">/data</span></p>
          </div>
          <StorageTabs activeTab={activeTab} onChange={setActiveTab} compact />
        </div>
        <div className="flex flex-wrap gap-3">
          {activeTab === 'browser' && isAdmin && (
            <button
              onClick={() => setShowNewFolderModal(true)}
              className="flex items-center gap-2 rounded-2xl bg-blue-600 px-4 py-2 text-sm font-bold text-white shadow-lg shadow-blue-600/20 transition hover:-translate-y-0.5 hover:bg-blue-700"
            >
              <Plus size={16} />
              New Folder
            </button>
          )}
          <button
            onClick={() => activeTab === 'browser' ? refetchListing() : refetchUsage()}
            className="flex items-center gap-2 rounded-2xl border border-gray-200 bg-white px-4 py-2 text-sm font-semibold text-gray-700 shadow-sm transition hover:-translate-y-0.5 hover:bg-gray-50 dark:border-gray-700 dark:bg-gray-800 dark:text-gray-200 dark:hover:bg-gray-700"
          >
            <RefreshCw size={16} className={(listingLoading || usageLoading) ? 'animate-spin' : ''} />
            Refresh
          </button>
        </div>
      </div>

      {activeTab === 'browser' && (
        <>
          <Breadcrumbs breadcrumbs={breadcrumbs} viewMode={browserViewMode} onViewModeChange={setBrowserViewMode} onNavigate={handleNavigate} />
          <div className={`grid gap-6 ${editorItem ? 'xl:grid-cols-[minmax(0,0.95fr)_minmax(460px,1.05fr)]' : 'grid-cols-1'}`}>
            <StorageBrowser
              listing={listing}
              isLoading={listingLoading}
              isAdmin={isAdmin}
              selectedItem={selectedItem}
              onNavigate={handleNavigate}
              onItemClick={handleItemClick}
              onDownload={handleDownloadItem}
              onRename={handleRequestRename}
              onDelete={handleRequestDelete}
              viewMode={browserViewMode}
            />
            {editorItem && (
              <StorageEditorPane
                item={editorItem}
                content={editorContent}
                isAdmin={isAdmin}
                isLoading={editorQuery.isLoading}
                error={editorQuery.error}
                isSaving={saveTextMutation.isPending}
                saveError={saveTextMutation.error}
                saveSuccess={saveTextMutation.isSuccess}
                onChange={setEditorContent}
                onSave={() => saveTextMutation.mutate()}
                onClose={handleCloseEditor}
              />
            )}
          </div>
          {listing && (
            <div className="text-sm font-medium text-gray-500 dark:text-gray-400">
              {folders} folder{folders !== 1 ? 's' : ''} · {files} file{files !== 1 ? 's' : ''}
            </div>
          )}
        </>
      )}

      {activeTab === 'windirstat' && (
        <WinDirStatView
          usage={usage}
          isLoading={usageLoading}
          error={usageError}
          onOpenDirectory={openWinDirStatDirectory}
        />
      )}

      {showNewFolderModal && (
        <NewFolderModal
          name={newFolderName}
          error={createDirMutation.error}
          isPending={createDirMutation.isPending}
          onChange={setNewFolderName}
          onCreate={handleCreateFolder}
          onClose={() => {
            setShowNewFolderModal(false)
            setNewFolderName('')
          }}
        />
      )}

      {showRenameModal && selectedItem && (
        <RenameModal
          item={selectedItem}
          value={renameValue}
          error={renameMutation.error}
          isPending={renameMutation.isPending}
          onChange={setRenameValue}
          onRename={() => renameMutation.mutate()}
          onClose={() => {
            setShowRenameModal(false)
            setRenameValue('')
          }}
        />
      )}

      {showDeleteConfirm && selectedItem && (
        <DeleteModal
          item={selectedItem}
          error={deleteMutation.error}
          isPending={deleteMutation.isPending}
          onDelete={handleDelete}
          onClose={() => {
            setShowDeleteConfirm(false)
            setSelectedItem(null)
          }}
        />
      )}
    </div>
  )
}

function StorageUnavailable() {
  return (
    <div className="space-y-6">
      <div className="relative overflow-hidden rounded-3xl border border-red-200 dark:border-red-500/30 bg-gradient-to-br from-red-50 via-white to-orange-50 dark:from-red-950/50 dark:via-gray-900 dark:to-orange-950/40 p-8 shadow-xl shadow-red-500/10">
        <div className="absolute -right-12 -top-12 h-40 w-40 rounded-full bg-red-400/20 blur-3xl" />
        <div className="relative">
          <p className="text-sm font-semibold uppercase tracking-[0.3em] text-red-600 dark:text-red-400">Storage</p>
          <h1 className="mt-2 text-3xl font-bold text-gray-950 dark:text-white">NFS Browser Offline</h1>
          <p className="mt-3 max-w-2xl text-gray-600 dark:text-gray-300">
            NFS storage is not configured or not accessible. To enable file browsing:
          </p>
          <ol className="mt-5 grid gap-3 text-sm text-gray-600 dark:text-gray-300 sm:grid-cols-3">
            <li className="rounded-2xl bg-white/70 dark:bg-gray-800/70 p-4 border border-red-100 dark:border-red-500/20">Configure managed NFS or external NFS</li>
            <li className="rounded-2xl bg-white/70 dark:bg-gray-800/70 p-4 border border-red-100 dark:border-red-500/20">Ensure `media-data` PVC is bound</li>
            <li className="rounded-2xl bg-white/70 dark:bg-gray-800/70 p-4 border border-red-100 dark:border-red-500/20">Restart the backend pod after mount</li>
          </ol>
        </div>
      </div>
    </div>
  )
}

function Breadcrumbs({ breadcrumbs, viewMode, onViewModeChange, onNavigate }: {
  breadcrumbs: { name: string; path: string }[]
  viewMode: BrowserViewMode
  onViewModeChange: (mode: BrowserViewMode) => void
  onNavigate: (path: string) => void
}) {
  return (
    <div className="rounded-3xl border border-gray-200 dark:border-gray-700 bg-white/90 dark:bg-gray-800/90 p-4 shadow-sm backdrop-blur">
      <div className="flex flex-col gap-3 lg:flex-row lg:items-center lg:justify-between">
        <div className="flex min-w-0 flex-wrap items-center gap-2">
          {breadcrumbs.map((crumb, index) => (
            <div key={crumb.path} className="flex items-center">
              {index > 0 && <ChevronRight size={16} className="text-gray-400 dark:text-gray-500 mx-1" />}
              <button
                onClick={() => onNavigate(crumb.path)}
                className={`flex items-center gap-2 rounded-2xl px-3 py-2 text-sm transition-all ${
                  index === breadcrumbs.length - 1
                    ? 'bg-blue-600 text-white shadow-lg shadow-blue-600/20'
                    : 'text-gray-500 dark:text-gray-400 hover:bg-gray-100 dark:hover:bg-gray-700 hover:text-gray-900 dark:hover:text-white'
                }`}
              >
                {index === 0 && <Home size={16} />}
                {crumb.name}
              </button>
            </div>
          ))}
        </div>
        <div className="flex w-fit rounded-2xl border border-gray-200 bg-white p-1 shadow-sm dark:border-gray-700 dark:bg-gray-900">
          <ViewModeButton mode="table" active={viewMode === 'table'} icon={<List size={16} />} label="List" onClick={onViewModeChange} />
          <ViewModeButton mode="thumbnail" active={viewMode === 'thumbnail'} icon={<Grid3X3 size={16} />} label="Thumbnails" onClick={onViewModeChange} />
        </div>
      </div>
    </div>
  )
}

function ViewModeButton({ mode, active, icon, label, onClick }: { mode: BrowserViewMode; active: boolean; icon: ReactNode; label: string; onClick: (mode: BrowserViewMode) => void }) {
  return (
    <button
      type="button"
      onClick={() => onClick(mode)}
      className={`flex items-center gap-1.5 rounded-xl px-3 py-2 text-sm font-bold transition ${active ? 'bg-blue-600 text-white shadow-lg shadow-blue-600/20' : 'text-gray-500 hover:bg-gray-100 hover:text-gray-900 dark:text-gray-400 dark:hover:bg-gray-700 dark:hover:text-white'}`}
      title={label}
    >
      {icon}
      <span className="hidden sm:inline">{label}</span>
    </button>
  )
}

function NewFolderModal({ name, error, isPending, onChange, onCreate, onClose }: {
  name: string
  error: unknown
  isPending: boolean
  onChange: (value: string) => void
  onCreate: () => void
  onClose: () => void
}) {
  return (
    <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
      <div className="bg-white dark:bg-gray-800 rounded-lg p-6 w-full max-w-md border border-gray-200 dark:border-gray-700">
        <h3 className="text-lg font-semibold text-gray-900 dark:text-white mb-4">Create New Folder</h3>
        <input
          type="text"
          value={name}
          onChange={(event) => onChange(event.target.value)}
          placeholder="Folder name"
          className="w-full px-4 py-2 bg-gray-100 dark:bg-gray-700 border border-gray-300 dark:border-gray-600 rounded text-gray-900 dark:text-white placeholder-gray-500 dark:placeholder-gray-400 focus:outline-none focus:border-blue-500"
          autoFocus
          onKeyDown={(event) => {
            if (event.key === 'Enter') onCreate()
            if (event.key === 'Escape') onClose()
          }}
        />
        {Boolean(error) && <p className="mt-2 text-sm text-red-500 dark:text-red-400">{(error as Error).message || 'Failed to create folder'}</p>}
        <div className="flex justify-end gap-3 mt-4">
          <button onClick={onClose} className="px-4 py-2 text-gray-500 dark:text-gray-400 hover:text-gray-900 dark:hover:text-white transition-colors">Cancel</button>
          <button onClick={onCreate} disabled={!name.trim() || isPending} className="px-4 py-2 bg-blue-600 hover:bg-blue-700 disabled:opacity-50 disabled:cursor-not-allowed rounded transition-colors">
            {isPending ? 'Creating...' : 'Create'}
          </button>
        </div>
      </div>
    </div>
  )
}

function RenameModal({ item, value, error, isPending, onChange, onRename, onClose }: {
  item: FileInfo
  value: string
  error: unknown
  isPending: boolean
  onChange: (value: string) => void
  onRename: () => void
  onClose: () => void
}) {
  return (
    <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
      <div className="bg-white dark:bg-gray-800 rounded-3xl p-6 w-full max-w-md border border-gray-200 dark:border-gray-700 shadow-2xl">
        <h3 className="text-lg font-black text-gray-900 dark:text-white mb-1">Rename {item.type === 'directory' ? 'Folder' : 'File'}</h3>
        <p className="mb-4 truncate font-mono text-xs text-gray-500 dark:text-gray-400">{item.path}</p>
        <input
          type="text"
          value={value}
          onChange={(event) => onChange(event.target.value)}
          className="w-full rounded-2xl border border-gray-300 bg-gray-100 px-4 py-3 text-gray-900 outline-none transition focus:border-indigo-500 dark:border-gray-600 dark:bg-gray-700 dark:text-white"
          autoFocus
          onKeyDown={(event) => {
            if (event.key === 'Enter' && value.trim()) onRename()
            if (event.key === 'Escape') onClose()
          }}
        />
        {Boolean(error) && <p className="mt-3 text-sm text-red-500 dark:text-red-400">{(error as Error).message || 'Failed to rename'}</p>}
        <div className="mt-5 flex justify-end gap-3">
          <button onClick={onClose} className="rounded-2xl px-4 py-2 text-gray-500 transition hover:bg-gray-100 hover:text-gray-900 dark:text-gray-400 dark:hover:bg-gray-700 dark:hover:text-white">Cancel</button>
          <button onClick={onRename} disabled={!value.trim() || value === item.name || isPending} className="rounded-2xl bg-indigo-600 px-4 py-2 font-bold text-white transition hover:bg-indigo-700 disabled:cursor-not-allowed disabled:opacity-50">
            {isPending ? 'Renaming...' : 'Rename'}
          </button>
        </div>
      </div>
    </div>
  )
}

function DeleteModal({ item, error, isPending, onDelete, onClose }: {
  item: FileInfo
  error: unknown
  isPending: boolean
  onDelete: () => void
  onClose: () => void
}) {
  return (
    <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
      <div className="bg-white dark:bg-gray-800 rounded-lg p-6 w-full max-w-md border border-gray-200 dark:border-gray-700">
        <h3 className="text-lg font-semibold text-gray-900 dark:text-white mb-4">Delete {item.type === 'directory' ? 'Folder' : 'File'}</h3>
        <p className="text-gray-600 dark:text-gray-300 mb-4">
          Are you sure you want to delete <span className="font-mono text-gray-900 dark:text-white">{item.name}</span>?
          {item.type === 'directory' && <span className="block mt-2 text-yellow-600 dark:text-yellow-400 text-sm">Note: Only empty directories can be deleted.</span>}
        </p>
        {Boolean(error) && <p className="mb-4 text-sm text-red-500 dark:text-red-400">{(error as Error).message || 'Failed to delete'}</p>}
        <div className="flex justify-end gap-3">
          <button onClick={onClose} className="px-4 py-2 text-gray-500 dark:text-gray-400 hover:text-gray-900 dark:hover:text-white transition-colors">Cancel</button>
          <button onClick={onDelete} disabled={isPending} className="px-4 py-2 bg-red-600 hover:bg-red-700 disabled:opacity-50 disabled:cursor-not-allowed rounded transition-colors">
            {isPending ? 'Deleting...' : 'Delete'}
          </button>
        </div>
      </div>
    </div>
  )
}
