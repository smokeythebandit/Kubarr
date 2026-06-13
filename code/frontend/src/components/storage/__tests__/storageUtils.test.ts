import { describe, expect, it } from 'vitest'
import type { FileInfo } from '../../../api/storage'
import { fileExtension, flattenUsage, highlightedHtml, isEditableFile, usagePercent } from '../storageUtils'

function file(path: string, size: number): FileInfo {
  return { name: path.split('/').pop() || path, path, type: 'file', size, modified: '2026-01-01T00:00:00Z', permissions: 'rw-r--r--' }
}

describe('storageUtils', () => {
  it('extracts lowercase file extensions', () => {
    expect(fileExtension('Media/Show.EPISODE.MKV')).toBe('mkv')
    expect(fileExtension('README')).toBe('')
  })

  it('allows known text files and small files in the editor', () => {
    expect(isEditableFile(file('config/settings.yaml', 200_000))).toBe(true)
    expect(isEditableFile(file('notes/unknown', 1024))).toBe(true)
    expect(isEditableFile(file('video/movie.mkv', 2_000_000))).toBe(false)
    expect(isEditableFile(null)).toBe(false)
  })

  it('escapes HTML before applying syntax highlighting', () => {
    const html = highlightedHtml('<script>alert("x")</script>', 'index.html')

    expect(html).toContain('&lt;script&gt;')
    expect(html).not.toContain('<script>')
  })

  it('flattens usage nodes depth-first', () => {
    const nodes = flattenUsage({
      name: 'Root',
      path: '',
      type: 'directory',
      size: 3,
      children: [
        { name: 'a.txt', path: 'a.txt', type: 'file', size: 1, children: [] },
        { name: 'dir', path: 'dir', type: 'directory', size: 2, children: [{ name: 'b.txt', path: 'dir/b.txt', type: 'file', size: 2, children: [] }] },
      ],
    })

    expect(nodes.map((node) => node.path)).toEqual(['', 'a.txt', 'dir', 'dir/b.txt'])
  })

  it('formats usage percentages', () => {
    expect(usagePercent(25, 100)).toBe('25.0%')
    expect(usagePercent(1, 0)).toBe('0%')
  })
})
