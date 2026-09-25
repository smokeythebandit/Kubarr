import { afterEach, describe, expect, it, vi } from 'vitest';
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import RegisterPage from './RegisterPage';

afterEach(() => vi.unstubAllGlobals());

async function submit() {
  fireEvent.change(screen.getByRole('textbox', { name: 'Username' }), { target: { value: 'new-user' } });
  fireEvent.change(screen.getByRole('textbox', { name: 'Email' }), { target: { value: 'new@example.test' } });
  fireEvent.change(screen.getByLabelText('Password'), { target: { value: 'test-only-password' } });
  await act(async () => {
    fireEvent.submit(screen.getByRole('button', { name: 'Register' }).closest('form')!);
  });
}

describe('registration', () => {
  it('sends an invite only to the registration endpoint and requires approval before offering login', async () => {
    const fetch = vi.fn().mockResolvedValue({ ok: true, json: async () => ({ status: 'pending' }) });
    vi.stubGlobal('fetch', fetch);
    render(<MemoryRouter initialEntries={['/login?register=1&invite=one-use-code']}><RegisterPage /></MemoryRouter>);
    await submit();
    await waitFor(() => expect(screen.getByRole('status').textContent).toContain('Awaiting admin approval'));
    expect(fetch).toHaveBeenCalledOnce();
    expect(fetch.mock.calls[0][0]).toBe('/auth/register');
    expect(JSON.parse(fetch.mock.calls[0][1].body)).toEqual({ username: 'new-user', email: 'new@example.test',
      password: 'test-only-password', invite_code: 'one-use-code' });
    expect(screen.queryByLabelText('Password')).toBeNull();
  });

  it('does not echo credentials or server details when registration is disabled', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue({ ok: false, status: 403 }));
    render(<MemoryRouter initialEntries={['/login?register=1']}><RegisterPage /></MemoryRouter>);
    await submit();
    await waitFor(() => expect(screen.getByRole('alert').textContent).toContain('disabled'));
    expect(screen.getByRole('alert').textContent).not.toContain('test-only-password');
  });
});
