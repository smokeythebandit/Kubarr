import { beforeEach, describe, expect, it, vi } from 'vitest';
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { MemoryRouter } from 'react-router-dom';
import type { ChatEvent } from '../../api/assistant';
import { AssistantButton, AssistantPanel } from './AssistantPanel';
import { BUDGET_PAUSED_TEXT } from './AssistantChat';
import { useAssistantPanel } from './assistantState';

const mocks = vi.hoisted(() => ({
  getStatus: vi.fn(),
  createConversation: vi.fn(),
  getConversation: vi.fn(),
  ask: vi.fn(),
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

function renderPanel() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <MemoryRouter initialEntries={['/apps']}>
        <AssistantButton />
        <AssistantPanel />
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

/** A controllable stream: the test pushes events and ends it. */
function controllableAsk() {
  let emit: (e: ChatEvent) => void = () => {};
  let end: () => void = () => {};
  let fail: (e: unknown) => void = () => {};
  mocks.ask.mockImplementation(
    (_id: string, _text: string, _ctx: unknown, onEvent: (e: ChatEvent) => void) =>
      new Promise<void>((resolve, reject) => {
        emit = onEvent;
        end = resolve;
        fail = reject;
      }),
  );
  return {
    emit: (e: ChatEvent) => act(() => emit(e)),
    end: () => act(async () => end()),
    fail: (e: unknown) => act(async () => fail(e)),
  };
}

async function openAndAsk(question: string) {
  fireEvent.click(await screen.findByRole('button', { name: 'Assistant' }));
  const input = await screen.findByLabelText('Ask the assistant');
  fireEvent.change(input, { target: { value: question } });
  fireEvent.keyDown(input, { key: 'Enter' });
  await waitFor(() => expect(mocks.ask).toHaveBeenCalled());
}

describe('AssistantPanel', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.permissions = new Set(['assistant.use']);
    useAssistantPanel.setState({ open: false, conversationId: null });
    mocks.getStatus.mockResolvedValue(status());
    mocks.createConversation.mockResolvedValue({ id: 'c1', title: 'New conversation', context: {}, created_at: '', updated_at: '' });
  });

  it('renders streamed deltas, status lines and sources', async () => {
    const stream = controllableAsk();
    renderPanel();
    await openAndAsk('How is Jellyfin?');

    expect(mocks.createConversation).toHaveBeenCalledWith({ page: '/apps' });
    expect(mocks.ask.mock.calls[0][0]).toBe('c1');
    expect(mocks.ask.mock.calls[0][2]).toEqual({ page: '/apps' });
    expect(screen.getByText('How is Jellyfin?')).toBeInTheDocument();

    stream.emit({ event: 'status', text: 'Looking at jellyfin…' });
    expect(screen.getByText('Looking at jellyfin…')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Stop' })).toBeInTheDocument();

    stream.emit({ event: 'text_delta', text: 'Jellyfin is ' });
    stream.emit({ event: 'text_delta', text: '**healthy**.' });
    expect(screen.queryByText('Looking at jellyfin…')).not.toBeInTheDocument();
    expect(screen.getByText('healthy').tagName).toBe('STRONG');

    stream.emit({ event: 'sources', items: [{ tool: 'get_app', target: 'jellyfin', label: 'App jellyfin' }] });
    stream.emit({ event: 'done', message_id: 9 });
    await stream.end();

    expect(screen.getByLabelText('Sources')).toHaveTextContent('App jellyfin');
    expect(screen.queryByRole('status')).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Send' })).toBeInTheDocument();
    expect(useAssistantPanel.getState().conversationId).toBe('c1');
  });

  it('marks a stream that ends without done as interrupted and can ask again', async () => {
    const stream = controllableAsk();
    renderPanel();
    await openAndAsk('Tell me everything');
    stream.emit({ event: 'text_delta', text: 'Partial answ' });
    await stream.end();

    expect(screen.getByText('Partial answ')).toBeInTheDocument();
    expect(screen.getByText(/Interrupted —/)).toBeInTheDocument();

    controllableAsk();
    fireEvent.click(screen.getByRole('button', { name: /ask again/ }));
    await waitFor(() => expect(mocks.ask).toHaveBeenCalledTimes(2));
    expect(mocks.ask.mock.calls[1][1]).toBe('Tell me everything');
    // The retry goes to the same conversation.
    expect(mocks.ask.mock.calls[1][0]).toBe('c1');
    expect(mocks.createConversation).toHaveBeenCalledTimes(1);
  });

  it('shows errors from the stream and refused requests', async () => {
    const { AssistantRequestError } = await import('../../api/assistant');
    const stream = controllableAsk();
    renderPanel();
    await openAndAsk('Hi');
    await stream.fail(new AssistantRequestError(429, 'Too many questions.'));
    expect(screen.getByRole('alert')).toHaveTextContent('Too many questions.');
  });

  it('shows the budget-paused message and disables input', async () => {
    mocks.getStatus.mockResolvedValue(status({ paused: true }));
    renderPanel();
    fireEvent.click(await screen.findByRole('button', { name: 'Assistant' }));
    expect(await screen.findByText(BUDGET_PAUSED_TEXT)).toBeInTheDocument();
    expect(screen.getByLabelText('Ask the assistant')).toBeDisabled();
  });

  it('toggles with Ctrl+J and closes with Escape', async () => {
    renderPanel();
    await screen.findByRole('button', { name: 'Assistant' });
    fireEvent.keyDown(document, { key: 'j', ctrlKey: true });
    expect(await screen.findByRole('dialog', { name: 'Assistant' })).toBeInTheDocument();
    fireEvent.keyDown(document, { key: 'Escape' });
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
  });

  it.each([
    ['not configured', { configured: false }],
    ['switched off', { enabled: false }],
    ['key unreadable', { key_error: true }],
  ])('is hidden when %s', async (_label, over) => {
    mocks.getStatus.mockResolvedValue(status(over));
    renderPanel();
    await waitFor(() => expect(mocks.getStatus).toHaveBeenCalled());
    await act(async () => {});
    expect(screen.queryByRole('button', { name: 'Assistant' })).not.toBeInTheDocument();
  });

  it('is hidden without assistant.use and does not ask for the status', async () => {
    mocks.permissions = new Set(['apps.view']);
    renderPanel();
    await act(async () => {});
    expect(screen.queryByRole('button', { name: 'Assistant' })).not.toBeInTheDocument();
    expect(mocks.getStatus).not.toHaveBeenCalled();
  });

  it('loads an existing conversation with an interrupted answer', async () => {
    useAssistantPanel.setState({ open: true, conversationId: 'c7' });
    mocks.getConversation.mockResolvedValue({
      id: 'c7', title: 't', context: {}, created_at: '', updated_at: '',
      messages: [
        { id: 1, role: 'user', text: 'Old question', sources: [], status: 'complete', created_at: '' },
        { id: 2, role: 'assistant', text: 'Half', sources: [], status: 'interrupted', created_at: '' },
      ],
    });
    renderPanel();
    expect(await screen.findByText('Old question')).toBeInTheDocument();
    expect(screen.getByText(/Interrupted —/)).toBeInTheDocument();
  });
});
