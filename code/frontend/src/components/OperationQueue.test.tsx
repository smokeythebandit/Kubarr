import { render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import type { AppOperation } from '../types'
import { OperationQueue } from './OperationQueue'

function operation(overrides: Partial<AppOperation>): AppOperation {
  const now = new Date().toISOString()
  return { id: crypto.randomUUID(), app_name: 'qbittorrent', operation: 'update', status: 'queued', message: null, error: null, attempts: 0, created_by: 1, created_at: now, started_at: null, finished_at: null, updated_at: now, ...overrides }
}

describe('OperationQueue', () => {
  it('renders running and queued worker operations', () => {
    render(<OperationQueue displayNames={{ qbittorrent: 'qBittorrent', sonarr: 'Sonarr' }} operations={[operation({ id: 'running', status: 'running', started_at: new Date().toISOString() }), operation({ id: 'queued', app_name: 'sonarr', operation: 'install' })]} />)
    expect(screen.getByText('qBittorrent')).toBeInTheDocument()
    expect(screen.getByText('Sonarr')).toBeInTheDocument()
    expect(screen.getByText('Running')).toBeInTheDocument()
    expect(screen.getByText('Queued #1')).toBeInTheDocument()
    expect(screen.getByRole('progressbar')).toBeInTheDocument()
  })

  it('shows recent failures but hides succeeded operations', () => {
    render(<OperationQueue displayNames={{ radarr: 'Radarr' }} operations={[operation({ id: 'failed', app_name: 'radarr', status: 'failed', error: 'Helm failed' }), operation({ id: 'done', status: 'succeeded' })]} />)
    expect(screen.getByText('Radarr')).toBeInTheDocument()
    expect(screen.getByText('Helm failed')).toBeInTheDocument()
    expect(screen.queryByText('qBittorrent')).not.toBeInTheDocument()
  })

  it('renders nothing when the queue has no visible operations', () => {
    const { container } = render(<OperationQueue displayNames={{}} operations={[operation({ status: 'succeeded' })]} />)
    expect(container).toBeEmptyDOMElement()
  })
})
