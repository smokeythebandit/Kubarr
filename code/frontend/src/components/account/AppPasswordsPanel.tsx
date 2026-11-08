import { useEffect, useState } from 'react'
import { Check, Copy, KeyRound, Loader2, Trash2 } from 'lucide-react'
import {
  createAppPassword,
  deleteAppPassword,
  listAppPasswords,
  type AppPasswordInfo,
  type CreatedAppPassword,
} from '../../api/users'

interface Props {
  /** With 2FA on, app passwords are the only way to sign in on TV/phone apps. */
  twoFactorEnabled: boolean
  formatRelativeTime: (iso: string) => string
}

export default function AppPasswordsPanel({ twoFactorEnabled, formatRelativeTime }: Props) {
  const [passwords, setPasswords] = useState<AppPasswordInfo[]>([])
  const [loading, setLoading] = useState(true)
  const [name, setName] = useState('')
  const [creating, setCreating] = useState(false)
  const [created, setCreated] = useState<CreatedAppPassword | null>(null)
  const [copied, setCopied] = useState(false)
  const [deleting, setDeleting] = useState<number | null>(null)
  const [error, setError] = useState('')

  useEffect(() => {
    listAppPasswords()
      .then(setPasswords)
      .catch(() => setError('Failed to load app passwords'))
      .finally(() => setLoading(false))
  }, [])

  const handleCreate = async () => {
    setError('')
    setCreating(true)
    try {
      const result = await createAppPassword(name.trim())
      setCreated(result)
      setCopied(false)
      setName('')
      setPasswords(prev => [result, ...prev])
    } catch (err: unknown) {
      const e = err as { response?: { data?: { detail?: string } } }
      setError(e.response?.data?.detail || 'Failed to create app password')
    } finally {
      setCreating(false)
    }
  }

  const handleDelete = async (id: number) => {
    setError('')
    setDeleting(id)
    try {
      await deleteAppPassword(id)
      setPasswords(prev => prev.filter(p => p.id !== id))
      if (created?.id === id) setCreated(null)
    } catch {
      setError('Failed to revoke app password')
    } finally {
      setDeleting(null)
    }
  }

  const handleCopy = async () => {
    if (!created) return
    try {
      await navigator.clipboard.writeText(created.password)
      setCopied(true)
    } catch {
      // Clipboard can be unavailable on plain HTTP; the password stays visible.
    }
  }

  return (
    <div className="bg-white dark:bg-gray-800 rounded-lg border border-gray-200 dark:border-gray-700 p-6">
      <h3 className="text-lg font-semibold text-gray-900 dark:text-white mb-2 flex items-center gap-2">
        <KeyRound size={20} className="text-emerald-500" />
        App Passwords
      </h3>
      <p className="text-sm text-gray-600 dark:text-gray-400 mb-4">
        Sign in to Jellyfin's TV and phone apps with your Kubarr username and an app password.
        {twoFactorEnabled
          ? ' Two-factor authentication is on, so your account password will not work there.'
          : ' Your account password also works; an app password lets you cut off one device without changing it.'}
      </p>

      {error && (
        <div className="mb-3 p-2 text-sm rounded bg-red-50 dark:bg-red-900/20 text-red-700 dark:text-red-400">
          {error}
        </div>
      )}

      {created && (
        <div className="mb-4 p-3 rounded-lg border border-emerald-200 dark:border-emerald-800 bg-emerald-50 dark:bg-emerald-900/20">
          <p className="text-sm text-emerald-800 dark:text-emerald-300 mb-2">
            Copy this password for <strong>{created.name}</strong> now. It will not be shown again.
          </p>
          <div className="flex items-center gap-2">
            <code className="flex-1 min-w-0 break-all px-2 py-1 rounded bg-white dark:bg-gray-900 text-sm font-mono">
              {created.password}
            </code>
            <button
              onClick={handleCopy}
              className="p-1.5 rounded hover:bg-emerald-100 dark:hover:bg-emerald-900/40"
              title="Copy"
            >
              {copied ? <Check size={16} className="text-emerald-600" /> : <Copy size={16} />}
            </button>
          </div>
        </div>
      )}

      <div className="flex gap-2 mb-4">
        <input
          type="text"
          value={name}
          maxLength={64}
          onChange={e => setName(e.target.value)}
          onKeyDown={e => e.key === 'Enter' && name.trim() && !creating && handleCreate()}
          placeholder="Device name, e.g. Living room TV"
          className="flex-1 min-w-0 px-3 py-2 border border-gray-300 dark:border-gray-600 rounded-lg bg-white dark:bg-gray-700 text-gray-900 dark:text-gray-100 focus:ring-2 focus:ring-blue-500 focus:border-transparent"
        />
        <button
          onClick={handleCreate}
          disabled={creating || !name.trim()}
          className="px-4 py-2 bg-blue-600 hover:bg-blue-700 disabled:bg-blue-400 disabled:cursor-not-allowed text-white rounded-lg font-medium flex items-center gap-2"
        >
          {creating && <Loader2 size={16} className="animate-spin" />}
          Create
        </button>
      </div>

      {loading ? (
        <div className="flex items-center gap-2 text-gray-500">
          <Loader2 size={16} className="animate-spin" />
          Loading...
        </div>
      ) : passwords.length === 0 ? (
        <p className="text-sm text-gray-500 dark:text-gray-400">No app passwords</p>
      ) : (
        <ul className="space-y-2">
          {passwords.map(p => (
            <li
              key={p.id}
              className="p-3 rounded-lg border border-gray-200 dark:border-gray-700 bg-gray-50 dark:bg-gray-900 flex items-center justify-between gap-2"
            >
              <div className="min-w-0">
                <div className="font-medium text-sm truncate">{p.name}</div>
                <div className="text-xs text-gray-500 dark:text-gray-400">
                  {p.last_used_at ? `Last used ${formatRelativeTime(p.last_used_at)}` : 'Never used'}
                </div>
              </div>
              <button
                onClick={() => handleDelete(p.id)}
                disabled={deleting === p.id}
                className="p-1.5 text-red-600 hover:bg-red-100 dark:hover:bg-red-900/30 rounded transition-colors"
                title="Revoke app password"
              >
                {deleting === p.id ? <Loader2 size={16} className="animate-spin" /> : <Trash2 size={16} />}
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  )
}
