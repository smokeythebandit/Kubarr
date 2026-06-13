import { ArrowUp, Download, File, Folder, Pencil, RefreshCw, Trash2 } from 'lucide-react'
import type { ReactNode } from 'react'
import type { DirectoryListing, FileInfo } from '../../api/storage'
import { formatBytes, formatDate } from '../../api/storage'
import { folderGradients } from './storageUtils'

export type BrowserViewMode = 'table' | 'thumbnail'

interface StorageBrowserProps {
  listing?: DirectoryListing
  isLoading: boolean
  isAdmin: boolean
  selectedItem: FileInfo | null
  onNavigate: (path: string) => void
  onItemClick: (item: FileInfo) => void
  onDownload: (item: FileInfo) => void
  onRename: (item: FileInfo) => void
  onDelete: (item: FileInfo) => void
  viewMode: BrowserViewMode
}

export function StorageBrowser({
  listing,
  isLoading,
  isAdmin,
  selectedItem,
  onNavigate,
  onItemClick,
  onDownload,
  onRename,
  onDelete,
  viewMode,
}: StorageBrowserProps) {
  const items = listing?.items ?? []

  return (
    <div className="overflow-hidden rounded-3xl border border-gray-200 dark:border-gray-700 bg-white dark:bg-gray-800 shadow-xl shadow-gray-200/50 dark:shadow-black/20">
      {isLoading ? (
        <div className="flex items-center justify-center h-64">
          <div className="flex items-center gap-3 text-gray-500 dark:text-gray-400">
            <RefreshCw size={18} className="animate-spin" />
            Loading shared storage...
          </div>
        </div>
      ) : viewMode === 'thumbnail' ? (
        <ThumbnailView
          listing={listing}
          items={items}
          isAdmin={isAdmin}
          selectedItem={selectedItem}
          onNavigate={onNavigate}
          onItemClick={onItemClick}
          onDownload={onDownload}
          onRename={onRename}
          onDelete={onDelete}
        />
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
              {listing?.parent !== undefined && listing?.parent !== null && (
                <tr
                  onClick={() => onNavigate(listing.parent!)}
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
                  onClick={() => onItemClick(item)}
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
                            onDownload(item)
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
                              onRename(item)
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
                              onDelete(item)
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
  )
}

function ThumbnailView({ listing, items, isAdmin, selectedItem, onNavigate, onItemClick, onDownload, onRename, onDelete }: {
  listing?: DirectoryListing
  items: FileInfo[]
  isAdmin: boolean
  selectedItem: FileInfo | null
  onNavigate: (path: string) => void
  onItemClick: (item: FileInfo) => void
  onDownload: (item: FileInfo) => void
  onRename: (item: FileInfo) => void
  onDelete: (item: FileInfo) => void
}) {
  return (
    <div className="max-h-[calc(100vh-18rem)] overflow-auto p-4">
      <div className="grid grid-cols-[repeat(auto-fill,minmax(150px,1fr))] gap-4">
        {listing?.parent !== undefined && listing?.parent !== null && (
          <button onClick={() => onNavigate(listing.parent!)} className="rounded-3xl border border-blue-100 bg-blue-50 p-4 text-left transition hover:-translate-y-0.5 hover:border-blue-300 dark:border-blue-500/20 dark:bg-blue-950/30">
            <div className="flex h-24 items-center justify-center rounded-2xl bg-blue-100 text-blue-600 dark:bg-blue-900/40 dark:text-blue-300"><ArrowUp size={34} /></div>
            <p className="mt-3 truncate font-black text-gray-900 dark:text-white">..</p>
            <p className="text-xs text-gray-500 dark:text-gray-400">Parent directory</p>
          </button>
        )}
        {items.map((item, index) => (
          <div key={item.path} className={`group rounded-3xl border p-3 transition hover:-translate-y-0.5 hover:shadow-lg ${selectedItem?.path === item.path ? 'border-blue-300 bg-blue-50 dark:border-blue-500/40 dark:bg-blue-950/30' : 'border-gray-100 bg-white dark:border-gray-700 dark:bg-gray-800'}`}>
            <button type="button" onClick={() => onItemClick(item)} className="w-full text-left">
              <div className={`flex h-28 items-center justify-center rounded-2xl ${item.type === 'directory' ? `bg-gradient-to-br ${folderGradients[index % folderGradients.length]} text-white` : 'bg-gray-100 text-gray-500 dark:bg-gray-700 dark:text-gray-300'}`}>
                {item.type === 'directory' ? <Folder size={42} /> : <File size={42} />}
              </div>
              <p className="mt-3 truncate font-black text-gray-900 dark:text-white" title={item.name}>{item.name}</p>
              <p className="text-xs text-gray-500 dark:text-gray-400">{item.type === 'directory' ? 'Folder' : formatBytes(item.size)}</p>
            </button>
            <ItemActions item={item} isAdmin={isAdmin} onDownload={onDownload} onRename={onRename} onDelete={onDelete} compact />
          </div>
        ))}
      </div>
      {items.length === 0 && <EmptyFolder />}
    </div>
  )
}

function ItemActions({ item, isAdmin, onDownload, onRename, onDelete, compact = false }: {
  item: FileInfo
  isAdmin: boolean
  onDownload: (item: FileInfo) => void
  onRename: (item: FileInfo) => void
  onDelete: (item: FileInfo) => void
  compact?: boolean
}) {
  return (
    <div className={`flex ${compact ? 'mt-3 justify-end' : 'justify-end'} gap-2 opacity-100 transition md:opacity-0 md:group-hover:opacity-100 md:group-focus-within:opacity-100`}>
      {item.type === 'file' && <IconAction label="Download" color="emerald" icon={<Download size={16} />} onClick={() => onDownload(item)} />}
      {isAdmin && <IconAction label="Rename" color="indigo" icon={<Pencil size={16} />} onClick={() => onRename(item)} />}
      {isAdmin && <IconAction label="Delete" color="red" icon={<Trash2 size={16} />} onClick={() => onDelete(item)} />}
    </div>
  )
}

function IconAction({ label, color, icon, onClick }: { label: string; color: 'emerald' | 'indigo' | 'red'; icon: ReactNode; onClick: () => void }) {
  const classes = {
    emerald: 'bg-emerald-100 text-emerald-700 hover:bg-emerald-600 dark:bg-emerald-900/40 dark:text-emerald-300',
    indigo: 'bg-indigo-100 text-indigo-700 hover:bg-indigo-600 dark:bg-indigo-900/40 dark:text-indigo-300',
    red: 'bg-red-100 text-red-700 hover:bg-red-600 dark:bg-red-900/40 dark:text-red-300',
  }
  return <button type="button" onClick={(event) => { event.stopPropagation(); onClick() }} className={`flex items-center gap-1.5 rounded-xl p-2 transition hover:text-white lg:px-3 ${classes[color]}`} title={label} aria-label={label}>{icon}<span className="hidden text-xs font-bold lg:inline">{label}</span></button>
}

function EmptyFolder() {
  return (
    <div className="flex flex-col items-center justify-center px-5 py-16 text-center text-gray-500 dark:text-gray-400">
      <div className="mb-4 rounded-3xl bg-gray-100 p-5 dark:bg-gray-700"><Folder size={48} className="opacity-50" /></div>
      <p className="font-bold text-gray-700 dark:text-gray-200">This folder is empty</p>
      <p className="mt-1 text-sm">Create a folder or upload content through your apps.</p>
    </div>
  )
}
