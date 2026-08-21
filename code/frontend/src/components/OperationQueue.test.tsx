import { render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import type { AppOperation } from '../types'
import { OperationQueue } from './OperationQueue'

function operation(overrides: Partial<AppOperation>): AppOperation {
  const now = new Date().toISOString()
  return {
    id: crypto.randomUUID(),
    app_name: 'qbittorrent',
    operation: 'update',
    status: 'queued',
    message: null,
    error: null,
    attempts: 0,
    created_by: 1,
    created_at: now,
    started_at: null,
    finished_at: null,
    updated_at: now,
    ...overrides,
  }
}

describe('OperationQueue', () => {
  it('renders running and queued worker operations', () => {
    render(
      <OperationQueue
        displayNames={{ qbittorrent: 'qBittorrent', sonarr: 'Sonarr' }}
        operations={[
          operation({ id: 'running', status: 'running', started_at: new Date().toISOString() }),
          operation({ id: 'queued', app_name: 'sonarr', operation: 'install' }),
        ]}
      />,
    )

    expect(screen.getByText('qBittorrent')).toBeInTheDocument()
    expect(screen.getByText('Sonarr')).toBeInTheDocument()
    expect(screen.getByText('Worker activity')).toBeInTheDocument()
    expect(screen.getByText('Waiting queue')).toBeInTheDocument()
    expect(screen.getByText('Install')).toBeInTheDocument()
    expect(screen.getByRole('progressbar')).toBeInTheDocument()
  })

  it('shows failed and succeeded operations in history', () => {
    render(
      <OperationQueue
        displayNames={{ radarr: 'Radarr', qbittorrent: 'qBittorrent' }}
        operations={[
          operation({ id: 'failed', app_name: 'radarr', status: 'failed', error: 'Helm failed' }),
          operation({ id: 'done', app_name: 'qbittorrent', status: 'succeeded' }),
        ]}
      />,
    )

    expect(screen.getByText('Radarr')).toBeInTheDocument()
    expect(screen.getByText('Helm failed')).toBeInTheDocument()
    expect(screen.getByText('qBittorrent')).toBeInTheDocument()
    expect(screen.getAllByText('Succeeded')).toHaveLength(2)
  })

  it('renders an idle workspace when no operations are active', () => {
    render(
      <OperationQueue displayNames={{}} operations={[operation({ status: 'succeeded' })]} />,
    )

    expect(screen.getByText('Worker is idle')).toBeInTheDocument()
    expect(screen.getByText('No operations are waiting.')).toBeInTheDocument()
  })
})
