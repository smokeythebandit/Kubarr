import { useEffect, useState } from 'react'
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { Folder, File, ChevronRight, Home, Plus, Trash2, RefreshCw, Download, ArrowUp, Code2, Save, X, Pencil } from 'lucide-react'
import { storageApi, formatBytes, formatDate } from '../api/storage'
import type { FileInfo } from '../api/storage'
import { useAuth } from '../contexts/AuthContext'

const folderGradients = [
  'from-sky-500 to-blue-600',
  'from-violet-500 to-fuchsia-600',
  'from-emerald-500 to-teal-600',
  'from-amber-500 to-orange-600',
]

const editableExtensions = new Set([
  'txt', 'md', 'json', 'yaml', 'yml', 'toml', 'xml', 'html', 'css', 'js', 'jsx', 'ts', 'tsx', 'rs', 'py', 'sh', 'conf', 'env', 'log', 'ini',
])

function fileExtension(path: string) {
  const name = path.split('/').pop() || ''
  const index = name.lastIndexOf('.')
  return index === -1 ? '' : name.slice(index + 1).toLowerCase()
}

function isEditableFile(item: FileInfo | null) {
  if (!item || item.type !== 'file') return false
  return editableExtensions.has(fileExtension(item.path)) || item.size < 64 * 1024
}

function escapeHtml(value: string) {
  return value
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#39;')
}

function highlightLine(line: string, extension: string) {
  let escaped = escapeHtml(line)

  if (['json', 'yaml', 'yml', 'toml'].includes(extension)) {
    escaped = escaped.replace(/^([\s-]*)([A-Za-z0-9_.-]+)(\s*:)/, '$1<span class="text-cyan-300">$2</span>$3')
  }

  escaped = escaped
    .replace(/(&quot;.*?&quot;|'.*?')/g, '<span class="text-emerald-300">$1</span>')
    .replace(/\b(true|false|null|None|Some|Ok|Err)\b/g, '<span class="text-violet-300">$1</span>')
    .replace(/\b(const|let|var|function|return|if|else|for|while|match|async|await|pub|fn|struct|enum|impl|use|import|export|from|class|def)\b/g, '<span class="text-pink-300">$1</span>')
    .replace(/\b(\d+(?:\.\d+)?)\b/g, '<span class="text-amber-300">$1</span>')
    .replace(/(#.*$|\/\/.*$)/g, '<span class="text-slate-500">$1</span>')

  return escaped || '&nbsp;'
}

function highlightedHtml(content: string, path: string) {
  const extension = fileExtension(path)
  return content
    .split('\n')
    .map((line, index) => `<div class="min-h-6"><span class="mr-4 inline-block w-10 select-none text-right text-slate-600">${index + 1}</span>${highlightLine(line, extension)}</div>`)
    .join('')
}

export default function StoragePage() {
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
      setSelectedItem(item)
      setShowRenameModal(false)
      setRenameValue('')
      if (wasEditingRenamedFile) {
        setEditorItem(item)
      }
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
    },
  })

  const handleNavigate = (path: string) => {
    setCurrentPath(path)
    setSelectedItem(null)
    handleCloseEditor()
  }

  const handleItemClick = (item: FileInfo) => {
    if (item.type === 'directory') {
      handleNavigate(item.path)
    } else {
      setSelectedItem(item)
      if (isEditableFile(item)) {
        handleOpenEditor(item)
      } else {
        handleCloseEditor()
      }
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

  const handleOpenEditor = (item: FileInfo) => {
    setEditorItem(item)
    setEditorContent('')
  }

  const handleCloseEditor = () => {
    setEditorItem(null)
    setEditorContent('')
  }

  const textFile = editorQuery.data

  useEffect(() => {
    if (textFile && editorItem) {
      setEditorContent(textFile.content)
    }
  }, [textFile, editorItem])

  // Build breadcrumb path segments
  const pathSegments = currentPath ? currentPath.split('/').filter(Boolean) : []
  const breadcrumbs = [
    { name: 'Root', path: '' },
    ...pathSegments.map((segment, index) => ({
      name: segment,
      path: pathSegments.slice(0, index + 1).join('/'),
    })),
  ]

  const isLoading = listingLoading
  const folders = listing?.items.filter((item) => item.type === 'directory').length ?? 0
  const files = listing?.items.filter((item) => item.type === 'file').length ?? 0

  if (listingError) {
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

  return (
    <div className="space-y-6">
      <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
        <div>
          <h1 className="text-2xl font-black tracking-tight text-gray-950 dark:text-white">Storage</h1>
          <p className="mt-1 text-sm text-gray-500 dark:text-gray-400">NFS volume at <span className="font-mono">/data</span></p>
        </div>
        <div className="flex flex-wrap gap-3">
          {isAdmin && (
            <button
              onClick={() => setShowNewFolderModal(true)}
              className="flex items-center gap-2 rounded-2xl bg-blue-600 px-4 py-2 text-sm font-bold text-white shadow-lg shadow-blue-600/20 transition hover:-translate-y-0.5 hover:bg-blue-700"
            >
              <Plus size={16} />
              New Folder
            </button>
          )}
          <button
            onClick={() => refetchListing()}
            className="flex items-center gap-2 rounded-2xl border border-gray-200 bg-white px-4 py-2 text-sm font-semibold text-gray-700 shadow-sm transition hover:-translate-y-0.5 hover:bg-gray-50 dark:border-gray-700 dark:bg-gray-800 dark:text-gray-200 dark:hover:bg-gray-700"
          >
            <RefreshCw size={16} className={listingLoading ? 'animate-spin' : ''} />
            Refresh
          </button>
        </div>
      </div>

      {/* Breadcrumb Navigation */}
      <div className="rounded-3xl border border-gray-200 dark:border-gray-700 bg-white/90 dark:bg-gray-800/90 p-4 shadow-sm backdrop-blur">
        <div className="flex items-center gap-2 flex-wrap">
          {breadcrumbs.map((crumb, index) => (
            <div key={crumb.path} className="flex items-center">
              {index > 0 && <ChevronRight size={16} className="text-gray-400 dark:text-gray-500 mx-1" />}
              <button
                onClick={() => handleNavigate(crumb.path)}
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
      </div>

      <div className={`grid gap-6 ${editorItem ? 'xl:grid-cols-[minmax(0,0.95fr)_minmax(460px,1.05fr)]' : 'grid-cols-1'}`}>
        {/* File Listing */}
        <div className="overflow-hidden rounded-3xl border border-gray-200 dark:border-gray-700 bg-white dark:bg-gray-800 shadow-xl shadow-gray-200/50 dark:shadow-black/20">
          {isLoading ? (
            <div className="flex items-center justify-center h-64">
              <div className="flex items-center gap-3 text-gray-500 dark:text-gray-400">
                <RefreshCw size={18} className="animate-spin" />
                Loading shared storage...
              </div>
            </div>
          ) : (
            <div className="overflow-x-auto">
          <table className="w-full min-w-[640px]">
            <thead className="bg-gray-50 dark:bg-gray-900/50">
              <tr>
                <th className="text-left px-5 py-4 text-xs font-black uppercase tracking-wider text-gray-500 dark:text-gray-400">Name</th>
                <th className="text-left px-5 py-4 text-xs font-black uppercase tracking-wider text-gray-500 dark:text-gray-400">Size</th>
                <th className="text-left px-5 py-4 text-xs font-black uppercase tracking-wider text-gray-500 dark:text-gray-400">Modified</th>
                <th className="text-left px-5 py-4 text-xs font-black uppercase tracking-wider text-gray-500 dark:text-gray-400">Permissions</th>
                <th className="px-5 py-4" />
              </tr>
            </thead>
            <tbody className="divide-y divide-gray-200 dark:divide-gray-700">
              {/* Parent directory row */}
              {listing?.parent !== undefined && listing?.parent !== null && (
                <tr
                  onClick={() => handleNavigate(listing.parent!)}
                  className="cursor-pointer transition-colors hover:bg-blue-50 dark:hover:bg-blue-950/30"
                >
                  <td className="px-5 py-4">
                    <div className="flex items-center gap-3">
                      <div className="rounded-2xl bg-blue-100 dark:bg-blue-900/40 p-2 text-blue-600 dark:text-blue-300">
                        <ArrowUp size={18} />
                      </div>
                      <div>
                        <span className="font-bold text-gray-900 dark:text-white">..</span>
                        <p className="text-xs text-gray-500 dark:text-gray-400">Parent directory</p>
                      </div>
                    </div>
                  </td>
                  <td className="px-5 py-4 text-gray-500 dark:text-gray-400">-</td>
                  <td className="px-5 py-4 text-gray-500 dark:text-gray-400">-</td>
                  <td className="px-5 py-4 text-gray-500 dark:text-gray-400">-</td>
                  <td className="px-5 py-4" />
                </tr>
              )}
              {listing?.items.map((item, index) => (
                <tr
                  key={item.path}
                  onClick={() => handleItemClick(item)}
                  className={`group cursor-pointer transition-all hover:bg-gray-50 dark:hover:bg-gray-700/50 ${
                    selectedItem?.path === item.path ? 'bg-blue-50 dark:bg-blue-900/30 ring-1 ring-inset ring-blue-300 dark:ring-blue-500/40' : ''
                  }`}
                >
                  <td className="px-5 py-4">
                    <div className="flex items-center gap-3">
                      {item.type === 'directory' ? (
                        <div className={`rounded-2xl bg-gradient-to-br ${folderGradients[index % folderGradients.length]} p-2.5 text-white shadow-lg shadow-blue-900/10 transition group-hover:scale-105`}>
                          <Folder size={20} />
                        </div>
                      ) : (
                        <div className="rounded-2xl bg-gray-100 dark:bg-gray-700 p-2.5 text-gray-500 dark:text-gray-300 transition group-hover:scale-105">
                          <File size={20} />
                        </div>
                      )}
                      <div className="min-w-0">
                        <span className="block truncate font-bold text-gray-900 dark:text-white">{item.name}</span>
                        <span className="text-xs capitalize text-gray-500 dark:text-gray-400">{item.type}</span>
                      </div>
                    </div>
                  </td>
                  <td className="px-5 py-4 text-sm font-medium text-gray-600 dark:text-gray-300">
                    {item.type === 'directory' ? '-' : formatBytes(item.size)}
                  </td>
                  <td className="px-5 py-4 text-sm text-gray-500 dark:text-gray-400">{formatDate(item.modified)}</td>
                  <td className="px-5 py-4">
                    <span className="rounded-xl bg-gray-100 dark:bg-gray-700 px-2.5 py-1 font-mono text-xs text-gray-600 dark:text-gray-300">{item.permissions}</span>
                  </td>
                  <td className="px-5 py-4">
                    <div className="flex justify-end gap-2 opacity-100 transition md:opacity-0 md:group-hover:opacity-100 md:group-focus-within:opacity-100">
                      {item.type === 'file' && (
                        <button
                          type="button"
                          onClick={(event) => {
                            event.stopPropagation()
                            handleDownloadItem(item)
                          }}
                          className="flex items-center gap-1.5 rounded-xl bg-emerald-100 p-2 text-emerald-700 transition hover:bg-emerald-600 hover:text-white dark:bg-emerald-900/40 dark:text-emerald-300 lg:px-3"
                          title="Download"
                          aria-label={`Download ${item.name}`}
                        >
                          <Download size={16} />
                          <span className="hidden text-xs font-bold lg:inline">Download</span>
                        </button>
                      )}
                      {isAdmin && (
                        <>
                          <button
                            type="button"
                          onClick={(event) => {
                            event.stopPropagation()
                            handleRequestRename(item)
                          }}
                            className="flex items-center gap-1.5 rounded-xl bg-indigo-100 p-2 text-indigo-700 transition hover:bg-indigo-600 hover:text-white dark:bg-indigo-900/40 dark:text-indigo-300 lg:px-3"
                            title="Rename"
                            aria-label={`Rename ${item.name}`}
                          >
                            <Pencil size={16} />
                            <span className="hidden text-xs font-bold lg:inline">Rename</span>
                          </button>
                          <button
                            type="button"
                            onClick={(event) => {
                              event.stopPropagation()
                              handleRequestDelete(item)
                            }}
                            className="flex items-center gap-1.5 rounded-xl bg-red-100 p-2 text-red-700 transition hover:bg-red-600 hover:text-white dark:bg-red-900/40 dark:text-red-300 lg:px-3"
                            title="Delete"
                            aria-label={`Delete ${item.name}`}
                          >
                            <Trash2 size={16} />
                            <span className="hidden text-xs font-bold lg:inline">Delete</span>
                          </button>
                        </>
                      )}
                    </div>
                  </td>
                </tr>
              ))}
              {listing?.items.length === 0 && (
                <tr>
                  <td colSpan={5} className="px-5 py-16 text-center text-gray-500 dark:text-gray-400">
                    <div className="flex flex-col items-center justify-center">
                      <div className="mb-4 rounded-3xl bg-gray-100 dark:bg-gray-700 p-5">
                        <Folder size={48} className="opacity-50" />
                      </div>
                      <p className="font-bold text-gray-700 dark:text-gray-200">This folder is empty</p>
                      <p className="mt-1 text-sm">Create a folder or upload content through your apps.</p>
                    </div>
                  </td>
                </tr>
              )}
            </tbody>
          </table>
            </div>
          )}
        </div>

        {editorItem && (
          <div className="overflow-hidden rounded-3xl border border-slate-700 bg-slate-950 shadow-2xl shadow-slate-950/30">
            <div className="flex flex-col gap-4 border-b border-white/10 bg-gradient-to-r from-slate-900 via-indigo-950 to-slate-900 p-5 sm:flex-row sm:items-center sm:justify-between">
              <div className="min-w-0">
                <p className="flex items-center gap-2 text-xs font-black uppercase tracking-[0.3em] text-cyan-300">
                  <Code2 size={14} />
                  Editor
                </p>
                <h2 className="mt-2 truncate text-2xl font-black text-white">{editorItem.name}</h2>
                <p className="mt-1 truncate font-mono text-xs text-slate-400">{editorItem.path}</p>
              </div>
              <div className="flex flex-wrap gap-2">
                {isAdmin && (
                  <button
                    onClick={() => saveTextMutation.mutate()}
                    disabled={saveTextMutation.isPending || editorQuery.isLoading}
                    className="flex items-center gap-2 rounded-2xl bg-emerald-500 px-4 py-2 text-sm font-black text-white shadow-lg shadow-emerald-950/30 transition hover:-translate-y-0.5 hover:bg-emerald-600 disabled:cursor-not-allowed disabled:opacity-60"
                  >
                    <Save size={16} />
                    {saveTextMutation.isPending ? 'Saving...' : 'Save'}
                  </button>
                )}
                <button
                  onClick={handleCloseEditor}
                  className="flex items-center gap-2 rounded-2xl border border-white/15 bg-white/10 px-4 py-2 text-sm font-bold text-white transition hover:bg-white/20"
                >
                  <X size={16} />
                  Close
                </button>
              </div>
            </div>

            {editorQuery.isLoading ? (
              <div className="flex h-[620px] items-center justify-center text-slate-300">
                <RefreshCw size={18} className="mr-3 animate-spin" />
                Loading editor...
              </div>
            ) : editorQuery.error ? (
              <div className="m-5 rounded-3xl border border-red-500/30 bg-red-950/40 p-6 text-red-100">
                {(editorQuery.error as Error).message || 'Failed to load file'}
              </div>
            ) : (
              <div className="relative h-[620px] overflow-auto bg-slate-950">
                <pre
                  aria-hidden="true"
                  className="pointer-events-none absolute inset-0 m-0 min-h-full whitespace-pre-wrap break-words p-4 font-mono text-sm leading-6 text-slate-200"
                  dangerouslySetInnerHTML={{ __html: highlightedHtml(editorContent, editorItem.path) }}
                />
                <textarea
                  value={editorContent}
                  onChange={(event) => setEditorContent(event.target.value)}
                  readOnly={!isAdmin}
                  spellCheck={false}
                  className="relative z-10 h-full min-h-full w-full resize-none whitespace-pre-wrap break-words bg-transparent p-4 pl-[4.5rem] font-mono text-sm leading-6 text-transparent caret-cyan-300 outline-none selection:bg-cyan-500/30 read-only:cursor-default"
                />
              </div>
            )}

            {(saveTextMutation.error || saveTextMutation.isSuccess) && (
              <div className="border-t border-white/10 px-5 py-3 text-sm">
                {saveTextMutation.error ? (
                  <span className="text-red-300">{(saveTextMutation.error as Error).message || 'Failed to save file'}</span>
                ) : (
                  <span className="text-emerald-300">Saved to NFS storage.</span>
                )}
              </div>
            )}
          </div>
        )}
      </div>

      {/* Item count */}
      {listing && (
        <div className="text-sm font-medium text-gray-500 dark:text-gray-400">
          {folders} folder{folders !== 1 ? 's' : ''} · {files} file{files !== 1 ? 's' : ''}
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

      {/* Rename Modal */}
      {showRenameModal && selectedItem && (
        <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
          <div className="bg-white dark:bg-gray-800 rounded-3xl p-6 w-full max-w-md border border-gray-200 dark:border-gray-700 shadow-2xl">
            <h3 className="text-lg font-black text-gray-900 dark:text-white mb-1">Rename {selectedItem.type === 'directory' ? 'Folder' : 'File'}</h3>
            <p className="mb-4 truncate font-mono text-xs text-gray-500 dark:text-gray-400">{selectedItem.path}</p>
            <input
              type="text"
              value={renameValue}
              onChange={(event) => setRenameValue(event.target.value)}
              className="w-full rounded-2xl border border-gray-300 bg-gray-100 px-4 py-3 text-gray-900 outline-none transition focus:border-indigo-500 dark:border-gray-600 dark:bg-gray-700 dark:text-white"
              autoFocus
              onKeyDown={(event) => {
                if (event.key === 'Enter' && renameValue.trim()) renameMutation.mutate()
                if (event.key === 'Escape') {
                  setShowRenameModal(false)
                  setRenameValue('')
                }
              }}
            />
            {renameMutation.error && (
              <p className="mt-3 text-sm text-red-500 dark:text-red-400">
                {(renameMutation.error as Error).message || 'Failed to rename'}
              </p>
            )}
            <div className="mt-5 flex justify-end gap-3">
              <button
                onClick={() => {
                  setShowRenameModal(false)
                  setRenameValue('')
                }}
                className="rounded-2xl px-4 py-2 text-gray-500 transition hover:bg-gray-100 hover:text-gray-900 dark:text-gray-400 dark:hover:bg-gray-700 dark:hover:text-white"
              >
                Cancel
              </button>
              <button
                onClick={() => renameMutation.mutate()}
                disabled={!renameValue.trim() || renameValue === selectedItem.name || renameMutation.isPending}
                className="rounded-2xl bg-indigo-600 px-4 py-2 font-bold text-white transition hover:bg-indigo-700 disabled:cursor-not-allowed disabled:opacity-50"
              >
                {renameMutation.isPending ? 'Renaming...' : 'Rename'}
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
