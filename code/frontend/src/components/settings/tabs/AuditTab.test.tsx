import { describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { AuditTab } from './AuditTab';
import SettingsPage from '../../../pages/SettingsPage';
import type { AuditLog } from '../../../api/audit';

const { getLogs, getStats } = vi.hoisted(() => ({ getLogs: vi.fn(), getStats: vi.fn() }));
vi.mock('../../../api/audit', () => ({ auditApi: { getLogs, getStats } }));
vi.mock('../../../contexts/AuthContext', () => ({
  useAuth: () => ({ isAdmin: false, hasPermission: (permission: string) => permission === 'audit.view' }),
}));

const log: AuditLog = {
  id: 1, timestamp: '2026-01-01T00:00:00Z', user_id: 1, username: 'alice',
  action: 'login_failed', resource_type: 'session', resource_id: null,
  details: null, ip_address: null, user_agent: null, success: false, error_message: null,
};

const props = {
  auditLogs: [log], auditStats: null, auditLoading: false, auditPage: 2,
  auditTotalPages: 3, auditTotal: 45, auditPerPage: 10,
  auditError: null, auditStatsError: null, auditFilter: {}, clearingLogs: false,
  canManage: false, onRefresh: vi.fn(), onClearOldLogs: vi.fn(),
  onAuditFilterChange: vi.fn(), setAuditPage: vi.fn(),
  formatAuditAction: (action: string) => action,
  getActionIcon: () => null,
};

describe('AuditTab', () => {
  it('shows server page size and hides manage control for view-only users', () => {
    render(<AuditTab {...props} />);
    expect(screen.getByText('Showing 11 to 11 of 45 entries')).toBeTruthy();
    expect(screen.queryByText('Clear Old Logs')).toBeNull();
    expect(screen.getByRole('option', { name: 'System Setting Changed' })).toBeTruthy();
    fireEvent.click(screen.getByText('Refresh'));
    expect(props.onRefresh).toHaveBeenCalled();
  });

  it('offers VPN audit actions and resource filters', () => {
    render(<AuditTab {...props} />);
    const [actionFilter, resourceFilter] = screen.getAllByRole('combobox');

    for (const [value, label] of [
      ['vpn_provider_created', 'VPN Provider Created'],
      ['vpn_provider_updated', 'VPN Provider Updated'],
      ['vpn_provider_deleted', 'VPN Provider Deleted'],
      ['vpn_assigned', 'VPN Assigned'],
      ['vpn_removed', 'VPN Removed'],
    ]) {
      expect(screen.getByRole('option', { name: label })).toHaveProperty('value', value);
    }
    expect(screen.getByRole('option', { name: 'VPN' })).toHaveProperty('value', 'vpn');

    fireEvent.change(actionFilter, { target: { value: 'vpn_provider_created' } });
    fireEvent.change(resourceFilter, { target: { value: 'vpn' } });
    expect(props.onAuditFilterChange).toHaveBeenNthCalledWith(1, 'action', 'vpn_provider_created');
    expect(props.onAuditFilterChange).toHaveBeenNthCalledWith(2, 'resource_type', 'vpn');
  });

  it('shows errors rather than a misleading empty state', () => {
    render(<AuditTab {...props} auditLogs={[]} auditError="Failed to load audit logs" auditStatsError="Failed to load audit statistics" canManage />);
    expect(screen.getAllByRole('alert')).toHaveLength(2);
    expect(screen.getByText('Clear Old Logs')).toBeTruthy();
    expect(screen.queryByText(/No audit logs match/)).toBeNull();
  });

  it('sends only safe positive integer user IDs and allows clearing the filter', () => {
    const onAuditFilterChange = vi.fn();
    const { rerender } = render(<AuditTab {...props} onAuditFilterChange={onAuditFilterChange} />);
    const input = screen.getByLabelText('User ID');
    fireEvent.change(input, { target: { value: '42' } });
    expect(onAuditFilterChange).toHaveBeenCalledWith('user_id', 42);
    for (const value of ['0', '-2', '1.5', '9007199254740992']) {
      fireEvent.change(input, { target: { value } });
    }
    expect(onAuditFilterChange).toHaveBeenCalledTimes(1);
    rerender(<AuditTab {...props} auditFilter={{ user_id: 42 }} onAuditFilterChange={onAuditFilterChange} />);
    fireEvent.change(screen.getByLabelText('User ID'), { target: { value: '' } });
    expect(onAuditFilterChange).toHaveBeenLastCalledWith('user_id', undefined);
  });

  it('converts local dates to UTC and displays stored UTC dates as local wall time', () => {
    const onAuditFilterChange = vi.fn();
    const local = '2026-06-15T13:45';
    const iso = new Date(local).toISOString();
    const { rerender } = render(<AuditTab {...props} onAuditFilterChange={onAuditFilterChange} />);
    fireEvent.change(screen.getByLabelText('From'), { target: { value: local } });
    expect(onAuditFilterChange).toHaveBeenCalledWith('from', iso);
    rerender(<AuditTab {...props} auditFilter={{ from: iso, to: iso }} onAuditFilterChange={onAuditFilterChange} />);
    expect(screen.getByLabelText('From')).toHaveProperty('value', local);
    expect(screen.getByLabelText('To')).toHaveProperty('value', local);
    fireEvent.change(screen.getByLabelText('To'), { target: { value: '' } });
    expect(onAuditFilterChange).toHaveBeenLastCalledWith('to', undefined);
  });

  it('blocks requests for reversed dates, shows the range error, and fetches again after clearing', async () => {
    getLogs.mockResolvedValue({ logs: [log], page: 1, total: 45, total_pages: 3, per_page: 20 });
    getStats.mockResolvedValue(null);
    render(<MemoryRouter initialEntries={['/settings?section=audit']}><SettingsPage /></MemoryRouter>);
    await waitFor(() => expect(getLogs).toHaveBeenCalledTimes(1));
    fireEvent.click(screen.getByRole('button', { name: 'Next' }));
    await waitFor(() => expect(getLogs).toHaveBeenCalledTimes(2));
    expect(getLogs).toHaveBeenLastCalledWith(expect.objectContaining({ page: 2 }));
    fireEvent.change(screen.getByLabelText('From'), { target: { value: '2026-06-16T13:45' } });
    await waitFor(() => expect(getLogs).toHaveBeenCalledTimes(3));
    expect(getLogs).toHaveBeenLastCalledWith(expect.objectContaining({ from: new Date('2026-06-16T13:45').toISOString(), page: 1 }));
    fireEvent.change(screen.getByLabelText('To'), { target: { value: '2026-06-15T13:45' } });
    expect(await screen.findByText('To must be on or after From.', { selector: '#audit-range-error' })).toBeTruthy();
    expect(getLogs).toHaveBeenCalledTimes(3);
    fireEvent.click(screen.getByRole('button', { name: 'Refresh' }));
    expect(getLogs).toHaveBeenCalledTimes(3);
    fireEvent.change(screen.getByLabelText('To'), { target: { value: '' } });
    await waitFor(() => expect(getLogs).toHaveBeenCalledTimes(4));
    expect(getLogs).toHaveBeenLastCalledWith(expect.objectContaining({ to: undefined, page: 1 }));
    expect(screen.queryByText('To must be on or after From.', { selector: '#audit-range-error' })).toBeNull();
  });
});
