import type { FileInfo, StorageUsageNode } from '../../api/storage'

export const folderGradients = [
  'from-sky-500 to-blue-600',
  'from-violet-500 to-fuchsia-600',
  'from-emerald-500 to-teal-600',
  'from-amber-500 to-orange-600',
]

export const treemapColors = [
  'from-cyan-500 to-blue-600',
  'from-violet-500 to-fuchsia-600',
  'from-emerald-500 to-teal-600',
  'from-amber-500 to-orange-600',
  'from-rose-500 to-red-600',
  'from-lime-500 to-green-600',
  'from-sky-500 to-indigo-600',
  'from-purple-500 to-pink-600',
]

const editableExtensions = new Set([
  'txt', 'md', 'json', 'yaml', 'yml', 'toml', 'xml', 'html', 'css', 'js', 'jsx', 'ts', 'tsx', 'rs', 'py', 'sh', 'conf', 'env', 'log', 'ini',
])

export function fileExtension(path: string) {
  const name = path.split('/').pop() || ''
  const index = name.lastIndexOf('.')
  return index === -1 ? '' : name.slice(index + 1).toLowerCase()
}

export function isEditableFile(item: FileInfo | null) {
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

export function highlightedHtml(content: string, path: string) {
  const extension = fileExtension(path)
  return content
    .split('\n')
    .map((line, index) => `<div class="min-h-6"><span class="mr-4 inline-block w-10 select-none text-right text-slate-600">${index + 1}</span>${highlightLine(line, extension)}</div>`)
    .join('')
}

export function flattenUsage(node: StorageUsageNode): StorageUsageNode[] {
  return [node, ...node.children.flatMap(flattenUsage)]
}

export function topUsageNodes(root: StorageUsageNode, limit = 48) {
  return root.children
    .flatMap((node) => (node.type === 'directory' ? [node, ...node.children] : [node]))
    .filter((node) => node.size > 0)
    .sort((a, b) => b.size - a.size)
    .slice(0, limit)
}

export function usagePercent(size: number, total: number) {
  if (!total) return '0%'
  return `${((size / total) * 100).toFixed(1)}%`
}
