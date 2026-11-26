import { beforeEach, describe, expect, it, vi } from 'vitest';
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { MemoryRouter } from 'react-router-dom';
import { AskAssistantButton, AssistantPanel } from './AssistantPanel';
import { useAssistantPanel } from './assistantState';
import { failedOperationContext, isFailedUpdateEvent } from '../notifications/notificationModel';

const mocks = vi.hoisted(() => ({
  getStatus: vi.fn(),
  createConversation: vi.fn(),
  getConversation: vi.fn(),
  ask: vi.fn(),
  getOperations: vi.fn(),
  permissions: new Set<string>(['assistant.use']),
}));

vi.mock('../../api/assistant', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../../api/assistant')>();
  return {
    ...actual,
    assistantApi: { getStatus: mocks.getStatus },
    assistantChatApi: {
      createConversation: mocks.createConversation,
      getConversation: mocks.getConversation,
      ask: mocks.ask,
      listConversations: vi.fn(async () => []),
      deleteConversation: vi.fn(),
    },
  };
});
vi.mock('../../api/apps', () => ({ appsApi: { getOperations: mocks.getOperations } }));
vi.mock('../../contexts/AuthContext', () => ({
  useAuth: () => ({ isAuthenticated: true, hasPermission: (p: string) => mocks.permissions.has(p) }),
}));

const status = (over: Record<string, unknown> = {}) => ({
  configured: true,
  key_error: false,
  enabled: true,
  model: 'claude-opus-5-5',
  effort: 'low',
  month_spend_usd: 1,
  monthly_budget_usd: 20,
  paused: false,
  ...over,
});

const QUESTION = "What's the state of Jellyfin? Anything I should look at?";

function renderEntry(resolveContext?: () => Promise<never>) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <MemoryRouter initialEntries={['/apps']}>
        <AskAssistantButton
          label="Ask about this app"
          context={{ kind: 'app', app: 'jellyfin' }}
          draft={QUESTION}
          resolveContext={resolveContext}
        />
        <AssistantPanel />
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

describe('AskAssistantButton', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.permissions = new Set(['assistant.use']);
    useAssistantPanel.setState({ open: false, conversationId: 'old', seed: null });
    mocks.getStatus.mockResolvedValue(status());
    mocks.getConversation.mockResolvedValue({ messages: [], actions: [] });
    mocks.createConversation.mockResolvedValue({ id: 'c1', title: 'New conversation', context: {}, created_at: '', updated_at: '' });
    mocks.ask.mockResolvedValue(undefined);
  });

  it.each([
    ['not configured', () => mocks.getStatus.mockResolvedValue(status({ configured: false }))],
    ['switched off', () => mocks.getStatus.mockResolvedValue(status({ enabled: false }))],
    ['key unreadable', () => mocks.getStatus.mockResolvedValue(status({ key_error: true }))],
    ['no permission', () => (mocks.permissions = new Set())],
  ])('is hidden when %s', async (_name, arrange) => {
    arrange();
    renderEntry();
    await act(async () => {
      await Promise.resolve();
    });
    expect(screen.queryByRole('button', { name: /Ask about this app/ })).not.toBeInTheDocument();
    if (mocks.permissions.has('assistant.use')) await waitFor(() => expect(mocks.getStatus).toHaveBeenCalled());
    expect(screen.queryByRole('button', { name: /Ask about this app/ })).not.toBeInTheDocument();
  });

  it('opens a new conversation with the context and an editable, unsent question', async () => {
    renderEntry();
    fireEvent.click(await screen.findByRole('button', { name: /Ask about this app/ }));

    const input = (await screen.findByLabelText('Ask the assistant')) as HTMLTextAreaElement;
    expect(input.value).toBe(QUESTION);
    expect(useAssistantPanel.getState().conversationId).toBeNull();
    expect(mocks.createConversation).not.toHaveBeenCalled();
    expect(mocks.ask).not.toHaveBeenCalled();

    fireEvent.change(input, { target: { value: `${QUESTION} Also the logs.` } });
    fireEvent.click(screen.getByRole('button', { name: 'Send' }));
    await waitFor(() => expect(mocks.ask).toHaveBeenCalled());
    expect(mocks.createConversation).toHaveBeenCalledWith({ page: '/apps', kind: 'app', app: 'jellyfin' });
    expect(mocks.ask.mock.calls[0][1]).toBe(`${QUESTION} Also the logs.`);
  });

  it('uses the context the click resolves', async () => {
    renderEntry(async () => ({ kind: 'operation', app: 'jellyfin', operation_id: 'op-1' }) as never);
    fireEvent.click(await screen.findByRole('button', { name: /Ask about this app/ }));
    await screen.findByLabelText('Ask the assistant');
    expect(useAssistantPanel.getState().seed?.context).toEqual({ kind: 'operation', app: 'jellyfin', operation_id: 'op-1' });
  });

  it('falls back to the plain context when resolving fails', async () => {
    renderEntry(async () => {
      throw new Error('down');
    });
    fireEvent.click(await screen.findByRole('button', { name: /Ask about this app/ }));
    await screen.findByLabelText('Ask the assistant');
    expect(useAssistantPanel.getState().seed?.context).toEqual({ kind: 'app', app: 'jellyfin' });
  });
});

describe('failed update notifications', () => {
  it('recognises the failure events', () => {
    for (const e of ['app_update_failed', 'app_update_rollback_failed', 'app_rollback_failed']) {
      expect(isFailedUpdateEvent(e)).toBe(true);
    }
    for (const e of ['app_updated', 'app_rolled_back', null, undefined]) expect(isFailedUpdateEvent(e)).toBe(false);
  });

  it("picks the app's failed operation closest to the notification", async () => {
    const op = (id: string, app: string, status: string, at: string) => ({ id, app_name: app, status, finished_at: at, updated_at: at });
    mocks.getOperations.mockResolvedValue([
      op('old', 'jellyfin', 'failed', '2026-11-01T00:00:00Z'),
      op('other', 'sonarr', 'failed', '2026-11-20T00:00:00Z'),
      op('ok', 'jellyfin', 'succeeded', '2026-11-20T00:00:00Z'),
      op('hit', 'jellyfin', 'failed', '2026-11-20T00:00:01Z'),
    ]);
    expect(await failedOperationContext('jellyfin', '2026-11-20T00:00:02Z')).toEqual({
      kind: 'operation',
      app: 'jellyfin',
      operation_id: 'hit',
    });
    mocks.getOperations.mockResolvedValue([]);
    expect(await failedOperationContext('jellyfin', '2026-11-20T00:00:02Z')).toEqual({ kind: 'app', app: 'jellyfin' });
  });
});
