import { useState, type FormEvent } from 'react';
import { Link, useSearchParams } from 'react-router-dom';
import { CheckCircle, Clock, Loader2, Ship, Ticket } from 'lucide-react';

// Only these server messages are safe and useful to show verbatim.
const KNOWN_ERRORS: Record<string, string> = {
  'Invalid or used invite': 'This invite link is invalid, expired or already used. Ask for a new one.',
  'Account already exists': 'That username or email is already taken.',
  'Invalid registration fields': 'Check the fields: username 3–64 characters, a valid email, and a password of at least 8 characters.',
};

const inputClass = 'appearance-none block w-full px-3 py-2 border border-gray-300 dark:border-gray-700 rounded-md placeholder-gray-400 dark:placeholder-gray-500 text-gray-900 dark:text-white bg-white dark:bg-gray-800 focus:outline-none focus:ring-1 focus:ring-blue-500 focus:border-blue-500 sm:text-sm';
const labelClass = 'block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1';

export default function RegisterPage() {
  const [params] = useSearchParams();
  const [status, setStatus] = useState<'approved' | 'pending' | null>(null);
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
        if (response.status === 403) {
          setError('Open registration is disabled; use an invite link.');
        } else if (response.status === 400) {
          const detail = await response.json().then((body: { detail?: string }) => body.detail).catch(() => undefined);
          setError((detail && KNOWN_ERRORS[detail]) || 'Registration failed. Check the fields or invite link.');
        } else {
          setError(response.status === 503 ? 'Registration is busy. Try again in a moment.' : 'Registration failed.');
        }
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

  return (
    <div className="min-h-screen bg-gray-50 dark:bg-gray-900 flex items-center justify-center px-4 py-12">
      <main className="max-w-md w-full space-y-8">
        <div className="text-center">
          <div className="flex justify-center mb-4">
            <Ship size={48} className="text-blue-500 dark:text-blue-400" strokeWidth={2} />
          </div>
          <h1 className="text-3xl font-extrabold text-gray-900 dark:text-white">Create an account</h1>
          <p className="mt-2 text-sm text-gray-500 dark:text-gray-400">Join this Kubarr server</p>
        </div>

        {status ? (
          <div role="status" className="rounded-lg border border-gray-200 dark:border-gray-700 bg-white dark:bg-gray-800 p-6 text-center space-y-3">
            {status === 'pending' ? (
              <>
                <Clock size={32} className="mx-auto text-yellow-500" />
                <p className="font-medium text-gray-900 dark:text-white">Awaiting admin approval.</p>
                <p className="text-sm text-gray-500 dark:text-gray-400">You can sign in once an administrator approves your account.</p>
              </>
            ) : (
              <>
                <CheckCircle size={32} className="mx-auto text-green-500" />
                <p className="font-medium text-gray-900 dark:text-white">Account created. You can sign in.</p>
              </>
            )}
          </div>
        ) : (
          <form onSubmit={submit} className="space-y-5">
            {invite && (
              <div className="flex items-center gap-2 rounded-md border border-blue-200 dark:border-blue-800 bg-blue-50 dark:bg-blue-900/30 px-4 py-3 text-sm text-blue-700 dark:text-blue-200">
                <Ticket size={16} className="shrink-0" />
                <span>You were invited. Pick a username and password to finish.</span>
              </div>
            )}
            {error && (
              <div role="alert" className="rounded-md bg-red-100 dark:bg-red-900/50 border border-red-300 dark:border-red-700 p-4 text-sm text-red-700 dark:text-red-200">
                {error}
              </div>
            )}
            <div>
              <label htmlFor="register-username" className={labelClass}>Username</label>
              <input id="register-username" name="username" autoComplete="username" autoFocus minLength={3} maxLength={64} required className={inputClass} />
            </div>
            <div>
              <label htmlFor="register-email" className={labelClass}>Email</label>
              <input id="register-email" name="email" type="email" autoComplete="email" required className={inputClass} />
            </div>
            <div>
              <label htmlFor="register-password" className={labelClass}>Password</label>
              <input id="register-password" name="password" type="password" autoComplete="new-password" minLength={8} required className={inputClass} />
              <p className="mt-1 text-xs text-gray-500 dark:text-gray-400">At least 8 characters.</p>
            </div>
            <button
              type="submit"
              disabled={busy}
              className="w-full flex justify-center items-center gap-2 py-2 px-4 text-sm font-medium rounded-md text-white bg-blue-600 hover:bg-blue-700 focus:outline-none focus:ring-2 focus:ring-offset-2 focus:ring-blue-500 dark:focus:ring-offset-gray-900 transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
            >
              {busy && <Loader2 size={16} className="animate-spin" />}
              {busy ? 'Registering...' : 'Register'}
            </button>
          </form>
        )}

        <p className="text-center text-sm text-gray-500 dark:text-gray-400">
          {status ? null : 'Already have an account? '}
          <Link to="/login" className="font-medium text-blue-600 dark:text-blue-400 hover:underline">Sign in</Link>
        </p>
      </main>
    </div>
  );
}
