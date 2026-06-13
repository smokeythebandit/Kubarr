import { Code2, RefreshCw, Save, X } from 'lucide-react'
import type { FileInfo } from '../../api/storage'
import { highlightedHtml } from './storageUtils'

interface StorageEditorPaneProps {
  item: FileInfo
  content: string
  isAdmin: boolean
  isLoading: boolean
  error: unknown
  isSaving: boolean
  saveError: unknown
  saveSuccess: boolean
  onChange: (content: string) => void
  onSave: () => void
  onClose: () => void
}

export function StorageEditorPane({
  item,
  content,
  isAdmin,
  isLoading,
  error,
  isSaving,
  saveError,
  saveSuccess,
  onChange,
  onSave,
  onClose,
}: StorageEditorPaneProps) {
  return (
    <div className="overflow-hidden rounded-3xl border border-slate-700 bg-slate-950 shadow-2xl shadow-slate-950/30">
      <div className="flex flex-col gap-4 border-b border-white/10 bg-gradient-to-r from-slate-900 via-indigo-950 to-slate-900 p-5 sm:flex-row sm:items-center sm:justify-between">
        <div className="min-w-0">
          <p className="flex items-center gap-2 text-xs font-black uppercase tracking-[0.3em] text-cyan-300">
            <Code2 size={14} />
            Editor
          </p>
          <h2 className="mt-2 truncate text-2xl font-black text-white">{item.name}</h2>
          <p className="mt-1 truncate font-mono text-xs text-slate-400">{item.path}</p>
        </div>
        <div className="flex flex-wrap gap-2">
          {isAdmin && (
            <button
              onClick={onSave}
              disabled={isSaving || isLoading}
              className="flex items-center gap-2 rounded-2xl bg-emerald-500 px-4 py-2 text-sm font-black text-white shadow-lg shadow-emerald-950/30 transition hover:-translate-y-0.5 hover:bg-emerald-600 disabled:cursor-not-allowed disabled:opacity-60"
            >
              <Save size={16} />
              {isSaving ? 'Saving...' : 'Save'}
            </button>
          )}
          <button
            onClick={onClose}
            className="flex items-center gap-2 rounded-2xl border border-white/15 bg-white/10 px-4 py-2 text-sm font-bold text-white transition hover:bg-white/20"
          >
            <X size={16} />
            Close
          </button>
        </div>
      </div>

      {isLoading ? (
        <div className="flex h-[620px] items-center justify-center text-slate-300">
          <RefreshCw size={18} className="mr-3 animate-spin" />
          Loading editor...
        </div>
      ) : error ? (
        <div className="m-5 rounded-3xl border border-red-500/30 bg-red-950/40 p-6 text-red-100">
          {(error as Error).message || 'Failed to load file'}
        </div>
      ) : (
        <div className="relative h-[620px] overflow-auto bg-slate-950">
          <pre
            aria-hidden="true"
            className="pointer-events-none absolute inset-0 m-0 min-h-full whitespace-pre-wrap break-words p-4 font-mono text-sm leading-6 text-slate-200"
            dangerouslySetInnerHTML={{ __html: highlightedHtml(content, item.path) }}
          />
          <textarea
            value={content}
            onChange={(event) => onChange(event.target.value)}
            readOnly={!isAdmin}
            spellCheck={false}
            className="relative z-10 h-full min-h-full w-full resize-none whitespace-pre-wrap break-words bg-transparent p-4 pl-[4.5rem] font-mono text-sm leading-6 text-transparent caret-cyan-300 outline-none selection:bg-cyan-500/30 read-only:cursor-default"
          />
        </div>
      )}

      {(saveError || saveSuccess) && (
        <div className="border-t border-white/10 px-5 py-3 text-sm">
          {saveError ? (
            <span className="text-red-300">{(saveError as Error).message || 'Failed to save file'}</span>
          ) : (
            <span className="text-emerald-300">Saved to NFS storage.</span>
          )}
        </div>
      )}
    </div>
  )
}
