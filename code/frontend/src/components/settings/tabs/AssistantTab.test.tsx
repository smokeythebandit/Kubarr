import { beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { AssistantTab } from './AssistantTab';

const mocks = vi.hoisted(() => ({
  getStatus: vi.fn(),
  getSettings: vi.fn(),
  saveSettings: vi.fn(),
  setKey: vi.fn(),
  removeKey: vi.fn(),
  testKey: vi.fn(),
  canManage: true,
}));

vi.mock('../../../api/assistant', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../../../api/assistant')>();
  return {
    ...actual,
    assistantApi: {
      getStatus: (...a: unknown[]) => mocks.getStatus(...a),
      getSettings: (...a: unknown[]) => mocks.getSettings(...a),
      saveSettings: (...a: unknown[]) => mocks.saveSettings(...a),
      setKey: (...a: unknown[]) => mocks.setKey(...a),
      removeKey: (...a: unknown[]) => mocks.removeKey(...a),
      testKey: (...a: unknown[]) => mocks.testKey(...a),
    },
  };
});

vi.mock('../../../contexts/AuthContext', () => ({
  useAuth: () => ({ hasPermission: (p: string) => (p === 'settings.view') || (p === 'settings.manage' && mocks.canManage) }),
}));

const settings = {
  enabled: true,
  model: 'claude-opus-5-5',
  effort: 'low',
  monthly_budget_usd: 20,
  retention_days: 30,
};

const status = {
  configured: false,
  key_error: false,
  enabled: true,
  model: 'claude-opus-5-5',
  effort: 'low',
  month_spend_usd: 0,
  monthly_budget_usd: 20,
  paused: false,
};

describe('AssistantTab', () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    mocks.canManage = true;
    mocks.getStatus.mockReset().mockResolvedValue(status);
    mocks.getSettings.mockReset().mockResolvedValue(settings);
    mocks.saveSettings.mockReset().mockImplementation(async (s) => s);
    mocks.setKey.mockReset().mockResolvedValue({ configured: true });
    mocks.removeKey.mockReset().mockResolvedValue({ configured: false });
    mocks.testKey.mockReset().mockResolvedValue({ ok: true });
  });

  it('renders the saved settings and an unset key', async () => {
    render(<AssistantTab />);
    expect(await screen.findByTestId('key-status')).toHaveTextContent('Not set');
    expect(screen.getByLabelText('Model')).toHaveValue('claude-opus-5-5');
    expect(screen.getByRole('option', { name: /Claude Sonnet 5.5/ })).toBeInTheDocument();
    expect(screen.getByRole('option', { name: /Claude Haiku 5.5/ })).toBeInTheDocument();
    expect(screen.getByLabelText('Effort')).toHaveValue('low');
    expect(screen.getByLabelText('Monthly budget (USD)')).toHaveValue(20);
    expect(screen.getByLabelText('Keep conversations (days)')).toHaveValue(30);
    expect(screen.getByRole('switch', { name: 'Enable assistant' })).toHaveAttribute('aria-checked', 'true');
    expect(screen.getByRole('button', { name: 'Save key' })).toBeDisabled();
    expect(screen.queryByRole('button', { name: 'Test key' })).not.toBeInTheDocument();
  });

  it('saves a key, clears the field and refreshes the status', async () => {
    mocks.getStatus.mockResolvedValueOnce(status).mockResolvedValueOnce({ ...status, configured: true });
    render(<AssistantTab />);
    const input = await screen.findByLabelText('API key');
    expect(input).toHaveAttribute('type', 'password');
    fireEvent.change(input, { target: { value: '  sk-ant-test-value  ' } });
    fireEvent.click(screen.getByRole('button', { name: 'Save key' }));
    await waitFor(() => expect(mocks.setKey).toHaveBeenCalledWith('sk-ant-test-value'));
    expect(await screen.findByText('API key saved.')).toBeVisible();
    expect(screen.getByLabelText('API key')).toHaveValue('');
    expect(screen.getByTestId('key-status')).toHaveTextContent('Configured');
    expect(screen.getByRole('button', { name: 'Replace key' })).toBeInTheDocument();
  });

  it('tests the key and shows the generic failure message', async () => {
    mocks.getStatus.mockResolvedValue({ ...status, configured: true });
    mocks.testKey.mockResolvedValue({ ok: false, error: 'Anthropic rejected the API key.' });
    render(<AssistantTab />);
    fireEvent.click(await screen.findByRole('button', { name: 'Test key' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('Key test failed: Anthropic rejected the API key.');
  });

  it('reports a working key', async () => {
    mocks.getStatus.mockResolvedValue({ ...status, configured: true });
    render(<AssistantTab />);
    fireEvent.click(await screen.findByRole('button', { name: 'Test key' }));
    expect(await screen.findByText('The key works.')).toBeVisible();
  });

  it('removes the key after confirmation', async () => {
    mocks.getStatus.mockResolvedValue({ ...status, configured: true });
    const confirm = vi.fn(() => true);
    vi.stubGlobal('confirm', confirm);
    render(<AssistantTab />);
    fireEvent.click(await screen.findByRole('button', { name: 'Remove key' }));
    expect(confirm).toHaveBeenCalled();
    await waitFor(() => expect(mocks.removeKey).toHaveBeenCalled());
    vi.unstubAllGlobals();
  });

  it('shows this month’s spend and the paused state', async () => {
    mocks.getStatus.mockResolvedValue({ ...status, configured: true, month_spend_usd: 20.5, paused: true });
    render(<AssistantTab />);
    expect(await screen.findByText('$20.50 of $20.00')).toBeVisible();
    expect(screen.getByRole('progressbar', { name: 'Spend this month' })).toHaveAttribute('aria-valuenow', '100');
    expect(screen.getByText(/The monthly budget is used up/)).toBeVisible();
  });

  it('warns at 80 % of the budget', async () => {
    mocks.getStatus.mockResolvedValue({ ...status, month_spend_usd: 17 });
    render(<AssistantTab />);
    expect(await screen.findByRole('progressbar', { name: 'Spend this month' })).toHaveAttribute('aria-valuenow', '85');
    expect(screen.getByText(/Over 80 %/)).toBeVisible();
  });

  it('saves the edited settings', async () => {
    render(<AssistantTab />);
    fireEvent.change(await screen.findByLabelText('Model'), { target: { value: 'claude-haiku-5-5' } });
    fireEvent.change(screen.getByLabelText('Effort'), { target: { value: 'high' } });
    fireEvent.change(screen.getByLabelText('Monthly budget (USD)'), { target: { value: '7.5' } });
    fireEvent.change(screen.getByLabelText('Keep conversations (days)'), { target: { value: '90' } });
    fireEvent.click(screen.getByRole('switch', { name: 'Enable assistant' }));
    fireEvent.click(screen.getByRole('button', { name: 'Save' }));
    await waitFor(() => expect(mocks.saveSettings).toHaveBeenCalledWith({
      enabled: false,
      model: 'claude-haiku-5-5',
      effort: 'high',
      monthly_budget_usd: 7.5,
      retention_days: 90,
    }));
    expect(await screen.findByText('Assistant settings saved.')).toBeVisible();
  });

  it('shows the server error when saving fails', async () => {
    mocks.saveSettings.mockRejectedValue({ response: { data: { detail: 'retention_days: must be between 1 and 365' } } });
    render(<AssistantTab />);
    fireEvent.click(await screen.findByRole('button', { name: 'Save' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('retention_days: must be between 1 and 365');
  });

  it('is read-only without settings.manage', async () => {
    mocks.canManage = false;
    mocks.getStatus.mockResolvedValue({ ...status, configured: true });
    render(<AssistantTab />);
    expect(await screen.findByTestId('key-status')).toHaveTextContent('Configured');
    expect(screen.queryByLabelText('API key')).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Test key' })).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Remove key' })).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Save' })).not.toBeInTheDocument();
    expect(screen.getByLabelText('Model')).toBeDisabled();
    expect(screen.getByRole('switch', { name: 'Enable assistant' })).toBeDisabled();
  });

  it('says when the key could not be read instead of "not set"', async () => {
    mocks.getStatus.mockResolvedValue({ ...status, key_error: true });
    render(<AssistantTab />);
    expect(await screen.findByTestId('key-status')).toHaveTextContent("Couldn't read the key");
    expect(screen.getByRole('alert')).toHaveTextContent(/Couldn.t read the key from its Kubernetes Secret/);
  });

  it('shows a load error', async () => {
    mocks.getSettings.mockRejectedValue({ message: 'boom' });
    render(<AssistantTab />);
    expect(await screen.findByText('boom')).toBeVisible();
  });
});
