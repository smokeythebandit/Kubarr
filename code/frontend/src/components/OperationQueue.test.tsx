import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import type { AppOperation } from '../types'
import { OperationQueue } from './OperationQueue'

function operation(overrides: Partial<AppOperation>): AppOperation {
  const now = new Date().toISOString()
  return {
    id: crypto.randomUUID(),
    app_name: 'qbittorrent',
    operation: 'update',
    status: 'queued',
    stop_requested: false,
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
          operation({ id: 'retried', app_name: 'radarr', status: 'retried', message: 'A replacement operation was queued.' }),
        ]}
      />,
    )

    expect(screen.getAllByText('Radarr')).toHaveLength(2)
    expect(screen.getByText('Helm failed')).toBeInTheDocument()
    expect(screen.getByText('qBittorrent')).toBeInTheDocument()
    expect(screen.getAllByText('Succeeded')).toHaveLength(2)
    expect(screen.getByText('Retried')).toBeInTheDocument()
    expect(screen.getByText('A replacement operation was queued.')).toBeInTheDocument()
  })

  it('renders an idle workspace when no operations are active', () => {
    render(
      <OperationQueue displayNames={{}} operations={[operation({ status: 'succeeded' })]} />,
    )

    expect(screen.getByText('Worker is idle')).toBeInTheDocument()
    expect(screen.getByText('No operations are waiting.')).toBeInTheDocument()
  })

  it('offers valid state actions and distinguishes paused and cancelled records', () => {
    const onAction = vi.fn()
    const items = [operation({ id: 'queued', status: 'queued' }), operation({ id: 'paused', status: 'paused' }),
      operation({ id: 'running', status: 'running' }), operation({ id: 'failed', status: 'failed' }),
      operation({ id: 'cancelled', status: 'cancelled' })]
    render(<OperationQueue operations={items} displayNames={{}} canManage={() => true} onAction={onAction} />)
    expect(screen.getByText(/Requests a cooperative stop, not a rollback/)).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Request stop qbittorrent update' })).toBeEnabled()
    expect(screen.getAllByRole('button', { name: /Cancel qbittorrent update/ })).toHaveLength(2)
    expect(screen.getAllByText('Cancelled')).toHaveLength(2)
    expect(screen.getByText(/Paused · created/)).toBeInTheDocument()
    expect(screen.getAllByRole('button', { name: 'Pause qbittorrent update' })).toHaveLength(1)
    fireEvent.click(screen.getByRole('button', { name: 'Resume qbittorrent update' }))
    expect(onAction).toHaveBeenCalledWith('resume', items[1])
    fireEvent.click(screen.getByRole('button', { name: 'Retry qbittorrent update' }))
    expect(onAction).toHaveBeenCalledWith('retry', items[3])
  })

  it('disables actions without install permission or while a request is pending', () => {
    const item = operation({ id: 'queued' })
    const { rerender } = render(<OperationQueue operations={[item]} displayNames={{}} />)
    expect(screen.getByRole('button', { name: 'Pause qbittorrent update' })).toBeDisabled()
    rerender(<OperationQueue operations={[item]} displayNames={{}} canManage={() => true} pendingId="queued" onAction={vi.fn()} />)
    expect(screen.getByRole('button', { name: 'Pause qbittorrent update' })).toBeDisabled()
    expect(screen.getByRole('button', { name: 'Cancel qbittorrent update' })).toBeDisabled()
  })

  it('uses per-operation permission and keeps pending actions identifiable', () => {
    const items = [operation({ id: 'delete', operation: 'delete' }), operation({ id: 'restart', operation: 'restart' })]
    render(<OperationQueue operations={items} displayNames={{}} canManage={item => item.operation === 'restart'} pendingId="restart" onAction={vi.fn()} />)
    expect(screen.getByRole('button', { name: 'Pause qbittorrent delete' })).toHaveAttribute('title', 'Requires apps.delete permission')
    expect(screen.getByRole('button', { name: 'Pause qbittorrent delete' })).toBeDisabled()
    expect(screen.getByRole('button', { name: 'Pause qbittorrent restart' })).toBeDisabled()
    expect(screen.getByText('Pause…')).toBeInTheDocument()
  })

  it('labels an in-flight stop request and disables another while pending', () => {
    const item = operation({ id: 'running', status: 'running' })
    const onAction = vi.fn()
    const { rerender } = render(<OperationQueue operations={[item]} displayNames={{}} canManage={() => true} onAction={onAction} />)
    const stop = screen.getByRole('button', { name: 'Request stop qbittorrent update' })
    fireEvent.click(stop)
    expect(onAction).toHaveBeenCalledWith('cancel', item)
    rerender(<OperationQueue operations={[item]} displayNames={{}} canManage={() => true} onAction={onAction} pendingId="running" />)
    expect(screen.getByRole('button', { name: 'Request stop qbittorrent update' })).toBeDisabled()
    expect(screen.getByText('Requesting stop…')).toBeInTheDocument()
  })

  it('shows accepted stop request as running and blocks duplicate requests', () => {
    const item = operation({ id: 'running', status: 'running', stop_requested: true, message: 'Stop requested' })
    const onAction = vi.fn()
    render(<OperationQueue operations={[item]} displayNames={{}} canManage={() => true} onAction={onAction} />)
    expect(screen.getByText('Stop requested; awaiting worker outcome')).toBeInTheDocument()
    expect(screen.getByText(/External changes may already have occurred/)).toBeInTheDocument()
    const stop = screen.getByRole('button', { name: 'Stop requested qbittorrent update' })
    expect(stop).toBeDisabled()
    fireEvent.click(stop)
    expect(onAction).not.toHaveBeenCalled()
    expect(screen.queryByText('Cancelled')).not.toBeInTheDocument()
  })
})
