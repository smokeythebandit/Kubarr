import { beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { MemoryRouter } from 'react-router-dom';
import AssistantPage from './AssistantPage';

const mocks = vi.hoisted(() => ({
  getStatus: vi.fn(),
  listConversations: vi.fn(),
  getConversation: vi.fn(),
  deleteConversation: vi.fn(),
  permissions: new Set<string>(['assistant.use']),
}));

vi.mock('../api/assistant', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../api/assistant')>();
  return {
    ...actual,
    assistantApi: { getStatus: mocks.getStatus },
    assistantChatApi: {
      listConversations: mocks.listConversations,
      getConversation: mocks.getConversation,
      deleteConversation: mocks.deleteConversation,
      createConversation: vi.fn(),
      ask: vi.fn(),
    },
  };
});
vi.mock('../contexts/AuthContext', () => ({
  useAuth: () => ({ isAuthenticated: true, hasPermission: (p: string) => mocks.permissions.has(p) }),
}));

const status = (over: Record<string, unknown> = {}) => ({
  configured: true, key_error: false, enabled: true, model: 'claude-opus-5-5', effort: 'low',
  month_spend_usd: 0, monthly_budget_usd: 20, paused: false, ...over,
});

const conversation = (id: string, title: string) => ({
  id, title, context: {}, created_at: new Date().toISOString(), updated_at: new Date().toISOString(),
});

function renderPage(entry = '/assistant') {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <MemoryRouter initialEntries={[entry]}>
        <AssistantPage />
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

describe('AssistantPage', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.permissions = new Set(['assistant.use']);
    mocks.getStatus.mockResolvedValue(status());
    mocks.listConversations.mockResolvedValue([conversation('a', 'Why is Jellyfin slow?'), conversation('b', 'Storage')]);
    mocks.getConversation.mockImplementation(async (id: string) => ({
      ...conversation(id, 'x'),
      messages: [{ id: 1, role: 'user', text: `question in ${id}`, sources: [], status: 'complete', created_at: '' }],
    }));
    mocks.deleteConversation.mockResolvedValue(undefined);
  });

  it('lists conversations and opens one', async () => {
    renderPage();
    fireEvent.click(await screen.findByText('Why is Jellyfin slow?'));
    expect(await screen.findByText('question in a')).toBeInTheDocument();
  });

  it('opens the conversation named in the URL', async () => {
    renderPage('/assistant?c=b');
    expect(await screen.findByText('question in b')).toBeInTheDocument();
  });

  it('deletes a conversation after confirming', async () => {
    renderPage();
    fireEvent.click(await screen.findByRole('button', { name: 'Delete conversation Storage' }));
    mocks.listConversations.mockResolvedValue([conversation('a', 'Why is Jellyfin slow?')]);
    fireEvent.click(screen.getByRole('button', { name: 'Delete' }));
    await waitFor(() => expect(mocks.deleteConversation).toHaveBeenCalledWith('b'));
    await waitFor(() => expect(screen.queryByText('Storage')).not.toBeInTheDocument());
  });

  it('explains why the assistant is unavailable', async () => {
    mocks.getStatus.mockResolvedValue(status({ configured: false }));
    mocks.permissions = new Set(['assistant.use', 'settings.manage']);
    renderPage();
    expect(await screen.findByText(/Settings → Assistant/)).toBeInTheDocument();
    expect(mocks.listConversations).not.toHaveBeenCalled();
  });

  it('tells users without the permission', async () => {
    mocks.permissions = new Set(['apps.view']);
    renderPage();
    expect(await screen.findByText(/do not have access/)).toBeInTheDocument();
  });
});
