import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { VpnProvider } from '../../api/vpn';
import { VpnProviderForm } from './VpnProviderForm';

const mocks = vi.hoisted(() => ({
  listSupportedProviders: vi.fn(),
  createProvider: vi.fn(),
}));

vi.mock('../../api/vpn', () => ({
  vpnApi: {
    listSupportedProviders: (...args: unknown[]) => mocks.listSupportedProviders(...args),
    createProvider: (...args: unknown[]) => mocks.createProvider(...args),
  },
  getVpnTypeLabel: (type: string) => type,
}));

describe('VpnProviderForm outbound subnet allowlist', () => {
  beforeEach(() => {
    mocks.listSupportedProviders.mockResolvedValue([]);
    mocks.createProvider.mockReset();
  });

  it('starts empty for a new provider and explains specific cluster CIDRs', async () => {
    mocks.createProvider.mockResolvedValue({});
    const onSave = vi.fn();
    render(<VpnProviderForm onSave={onSave} onClose={vi.fn()} />);
    await act(async () => {});

    const subnetInput = screen.getByRole('textbox', { name: 'Allowed Subnets' });
    expect(subnetInput).toHaveValue('');
    expect(screen.getByText(/Allowlist only the specific cluster CIDRs/)).toBeVisible();
    fireEvent.change(screen.getByPlaceholderText('My VPN'), { target: { value: 'VPN' } });
    fireEvent.change(screen.getByPlaceholderText('Enter WireGuard private key'), { target: { value: 'test-key' } });
    fireEvent.click(screen.getByRole('button', { name: 'Add Provider' }));

    await waitFor(() => expect(mocks.createProvider).toHaveBeenCalledWith(expect.objectContaining({ firewall_outbound_subnets: '' })));
    expect(onSave).toHaveBeenCalled();
  });

  it('keeps an existing provider’s saved subnet list when editing', async () => {
    const provider = {
      id: 1, name: 'VPN', vpn_type: 'wireguard', service_provider: 'custom',
      enabled: true, kill_switch: true, firewall_outbound_subnets: '10.42.0.0/16',
    } as VpnProvider;
    render(<VpnProviderForm provider={provider} onSave={vi.fn()} onClose={vi.fn()} />);
    await act(async () => {});

    expect(screen.getByRole('textbox', { name: 'Allowed Subnets' })).toHaveValue('10.42.0.0/16');
  });
});
