import { describe, expect, it } from 'vitest'
import type { StorageUsageNode } from '../../../api/storage'
import { buildDirectoryRects, buildExtensionStats, buildTreemapRects, buildTreeRows, expandAncestors, extensionFor, isDescendantPath, visiblePercent } from '../storageStatistics'

const usageRoot: StorageUsageNode = {
  name: 'Root',
  path: '',
  type: 'directory',
  size: 100,
  children: [
    {
      name: 'Movies',
      path: 'Movies',
      type: 'directory',
      size: 70,
      children: [
        { name: 'big.mkv', path: 'Movies/big.mkv', type: 'file', size: 60, children: [] },
        { name: 'poster.jpg', path: 'Movies/poster.jpg', type: 'file', size: 10, children: [] },
      ],
    },
    { name: 'readme', path: 'readme', type: 'file', size: 30, children: [] },
    { name: 'empty', path: 'empty', type: 'directory', size: 0, children: [] },
  ],
}

describe('storageStatistics', () => {
  it('builds expanded tree rows sorted by size and skips zero-size children', () => {
    const rows = buildTreeRows(usageRoot, new Set(['']))

    expect(rows.map((row) => row.node.path)).toEqual(['', 'Movies', 'readme'])
    expect(rows.map((row) => row.depth)).toEqual([0, 1, 1])
  })

  it('expands ancestors for a selected path', () => {
    expect([...expandAncestors('Movies/Trailers/clip.mp4', new Set(['Existing']))]).toEqual(['Existing', '', 'Movies', 'Movies/Trailers'])
  })

  it('aggregates extension stats by size', () => {
    const stats = buildExtensionStats([
      { name: 'one.mkv', path: 'one.mkv', type: 'file', size: 30, children: [] },
      { name: 'two.mkv', path: 'two.mkv', type: 'file', size: 20, children: [] },
      { name: 'README', path: 'README', type: 'file', size: 10, children: [] },
    ])

    expect(stats.map(({ extension, size, files }) => ({ extension, size, files }))).toEqual([
      { extension: '.mkv', size: 50, files: 2 },
      { extension: '[none]', size: 10, files: 1 },
    ])
  })

  it('builds treemap rectangles for files and directories', () => {
    expect(buildTreemapRects(usageRoot).map((rect) => rect.node.path).sort()).toEqual(['Movies/big.mkv', 'Movies/poster.jpg', 'readme'])
    expect(buildDirectoryRects(usageRoot).map((rect) => rect.node.path)).toContain('Movies')
  })

  it('handles extension and path helpers', () => {
    expect(extensionFor('archive.tar.gz')).toBe('.gz')
    expect(extensionFor('.hidden')).toBe('[none]')
    expect(isDescendantPath('Movies/big.mkv', 'Movies')).toBe(true)
    expect(isDescendantPath('Music/song.flac', 'Movies')).toBe(false)
    expect(visiblePercent(1, 10_000)).toBe('<0.1%')
  })
})
