import { useState, type FormEvent } from 'react';
import { Link, useSearchParams } from 'react-router-dom';

export default function RegisterPage() {
  const [params] = useSearchParams();
  const [status, setStatus] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const invite = params.get('invite');

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const fields = new FormData(event.currentTarget);
    setBusy(true);
    setError(null);
    try {
      const response = await fetch('/auth/register', {
        method: 'POST', credentials: 'same-origin', headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ username: fields.get('username'), email: fields.get('email'),
          password: fields.get('password'), invite_code: invite }),
      });
      if (!response.ok) {
        setError(response.status === 403 ? 'Open registration is disabled; use an invite link.' :
          response.status === 400 ? 'Registration failed. Check the fields or invite link.' : 'Registration failed.');
        return;
      }
      const result = await response.json() as { status: 'approved' | 'pending' };
      setStatus(result.status);
    } catch {
      setError('Registration is unavailable.');
    } finally {
      setBusy(false);
    }
  }

  return <main className="min-h-screen bg-gray-900 text-white flex items-center justify-center p-6">
    <div className="max-w-md w-full space-y-5">
      <h1 className="text-2xl font-bold">Create an account</h1>
      {status ? <p role="status">{status === 'pending' ? 'Awaiting admin approval.' : 'Account created. You can sign in.'}</p> :
        <form onSubmit={submit} className="space-y-4">
          <label className="block">Username<input name="username" minLength={3} maxLength={64} required className="block w-full p-2 text-gray-900" /></label>
          <label className="block">Email<input name="email" type="email" required className="block w-full p-2 text-gray-900" /></label>
          <label className="block">Password<input name="password" type="password" minLength={8} required className="block w-full p-2 text-gray-900" /></label>
          {invite && <p>Using an invite link</p>}
          {error && <p role="alert">{error}</p>}
          <button type="submit" disabled={busy} className="bg-blue-600 p-2 rounded disabled:opacity-50">{busy ? 'Registering...' : 'Register'}</button>
        </form>}
      <Link to="/login" className="block text-blue-400">Sign in</Link>
    </div>
  </main>;
}
