import { beforeEach, describe, expect, it, vi } from 'vitest';
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { useState } from 'react';
import type { ActionCard } from '../../api/assistant';
import { ActionCardView, formatRemaining } from './ActionCardView';
import { withActions } from './AssistantChat';

const mocks = vi.hoisted(() => ({ runAction: vi.fn(), cancelAction: vi.fn() }));

vi.mock('../../api/assistant', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../../api/assistant')>();
  return {
    ...actual,
    assistantChatApi: { runAction: mocks.runAction, cancelAction: mocks.cancelAction },
  };
});

const NOW = Date.parse('2026-11-26T12:00:00Z');

const card = (over: Partial<ActionCard> = {}): ActionCard => ({
  id: 'a1',
  kind: 'update_app',
  app: 'jellyfin',
  message_id: 2,
  summary: {
    title: 'Update jellyfin',
    effect: 'Updates jellyfin from 1.0.0 to 1.1.0 (minor update).',
    from_version: '1.0.0',
    to_version: '1.1.0',
  },
  status: 'pending',
  expires_at: new Date(NOW + 5 * 60_000).toISOString(),
  ...over,
});

/** Holds the card in state like the chat does. */
function Harness({ initial, now = () => NOW }: { initial: ActionCard; now?: () => number }) {
  const [current, setCurrent] = useState(initial);
  return (
    <MemoryRouter>
      <ActionCardView card={current} onChange={setCurrent} now={now} />
    </MemoryRouter>
  );
}

describe('ActionCardView', () => {
  beforeEach(() => {
    mocks.runAction.mockReset();
    mocks.cancelAction.mockReset();
  });

  it('shows what will happen, the versions and the time left', () => {
    render(<Harness initial={card()} />);
    expect(screen.getByText('Update jellyfin')).toBeInTheDocument();
    expect(screen.getByText(/from 1.0.0 to 1.1.0/)).toBeInTheDocument();
    expect(screen.getByText('1.0.0 → 1.1.0')).toBeInTheDocument();
    expect(screen.getByRole('link', { name: 'jellyfin' })).toHaveAttribute('href', '/apps');
    expect(screen.getByText('Expires in 5:00')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Run' })).toBeEnabled();
    expect(screen.getByRole('button', { name: 'Cancel' })).toBeEnabled();
  });

  it('runs once: Run is disabled while running, then shows the queued operation', async () => {
    let finish: (v: unknown) => void = () => {};
    mocks.runAction.mockReturnValue(new Promise((resolve) => (finish = resolve)));
    render(<Harness initial={card()} />);
    fireEvent.click(screen.getByRole('button', { name: 'Run' }));
    expect(screen.getByRole('button', { name: 'Run' })).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Cancel' })).toBeDisabled();
    fireEvent.click(screen.getByRole('button', { name: 'Run' }));
    expect(mocks.runAction).toHaveBeenCalledTimes(1);
    await act(async () => finish({ status: 'running', operation_id: 'op-1' }));
    expect(screen.queryByRole('button', { name: 'Run' })).not.toBeInTheDocument();
    expect(screen.getByText(/Queued as an operation/)).toBeInTheDocument();
    expect(screen.getByRole('link', { name: 'View operations' })).toHaveAttribute('href', '/apps?tab=operations');
  });

  it('shows a refusal from the server', async () => {
    mocks.runAction.mockRejectedValue({ response: { data: { detail: 'You need the apps.install permission for this.' } } });
    render(<Harness initial={card()} />);
    fireEvent.click(screen.getByRole('button', { name: 'Run' }));
    await waitFor(() => expect(screen.getByText(/You need the apps.install permission/)).toBeInTheDocument());
    expect(screen.queryByRole('button', { name: 'Run' })).not.toBeInTheDocument();
  });

  it('cancels', async () => {
    mocks.cancelAction.mockResolvedValue(undefined);
    render(<Harness initial={card()} />);
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    await waitFor(() => expect(screen.getByText('Cancelled.')).toBeInTheDocument());
    expect(mocks.runAction).not.toHaveBeenCalled();
  });

  it('shows a cancel error and keeps the card runnable', async () => {
    mocks.cancelAction.mockRejectedValue({ response: { data: { detail: 'This action was already run.' } } });
    render(<Harness initial={card()} />);
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    await waitFor(() => expect(screen.getByRole('alert')).toHaveTextContent('This action was already run.'));
  });

  it('expires when the countdown runs out', () => {
    render(<Harness initial={card({ expires_at: new Date(NOW - 1000).toISOString() })} />);
    expect(screen.queryByRole('button', { name: 'Run' })).not.toBeInTheDocument();
    expect(screen.getByText(/Expired/)).toBeInTheDocument();
  });

  it.each([
    ['done', /Done\./],
    ['failed', /Did not complete: Not healthy/],
    ['cancelled', /Cancelled\./],
    ['expired', /Expired/],
    ['running', /Queued as an operation/],
  ] as const)('reloads in the %s state', (status, text) => {
    render(<Harness initial={card({ status, operation_id: 'op-1', error: status === 'failed' ? 'Not healthy' : undefined })} />);
    expect(screen.getByText(text)).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Run' })).not.toBeInTheDocument();
  });

  it('formats the remaining time', () => {
    expect(formatRemaining(600_000)).toBe('10:00');
    expect(formatRemaining(61_500)).toBe('1:02');
    expect(formatRemaining(-5)).toBe('0:00');
  });
});

describe('withActions', () => {
  it('attaches each card to the answer that proposed it', () => {
    const base = { sources: [], status: 'complete' as const, created_at: '' };
    const shown = withActions(
      [
        { id: 1, role: 'user', text: 'q', ...base },
        { id: 2, role: 'assistant', text: 'a', ...base },
        { id: 3, role: 'assistant', text: 'outcome', ...base },
      ],
      [card(), card({ id: 'a2', message_id: undefined })],
    );
    expect(shown[1].actions?.map((a) => a.id)).toEqual(['a1']);
    expect(shown[2].actions?.map((a) => a.id)).toEqual(['a2']);
  });
});
