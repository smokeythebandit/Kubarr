import { useState, FormEvent, useEffect, useRef, useMemo } from 'react'
import { Ship, Shield, ArrowLeft, Loader2, User, Check, Key } from 'lucide-react'
import { sessionLogin, verify2FA, loginWithRecoveryCode, SessionLoginResponse, getAccounts, switchAccount, AccountInfo } from '../api/auth'
import { getCurrentUser } from '../api/users'
import { oauthApi, AvailableProvider } from '../api/oauth'
import { precacheDashboard } from '../utils/precache'

type LoginStep = 'credentials' | '2fa_required' | '2fa_setup_required' | 'recovery_code'

export default function LoginPage() {
  const [username, setUsername] = useState('')
  const [password, setPassword] = useState('')
  const [error, setError] = useState<string | null>(null)
  const [loading, setLoading] = useState(false)
  const [checkingSession, setCheckingSession] = useState(true)

  // 2FA state
  const [step, setStep] = useState<LoginStep>('credentials')
  const [totpCode, setTotpCode] = useState('')
  const totpInputRef = useRef<HTMLInputElement>(null)
  const [recoveryCode, setRecoveryCode] = useState('')

  // OAuth providers
  const [oauthProviders, setOauthProviders] = useState<AvailableProvider[]>([])

  // Multi-account state
  const [existingAccounts, setExistingAccounts] = useState<AccountInfo[]>([])
  const [showAccountPicker, setShowAccountPicker] = useState(false)
  const [switchingAccount, setSwitchingAccount] = useState(false)

  // Capture OAuth parameters from URL for redirect after login
  const oauthParams = useMemo(() => {
    const params = new URLSearchParams(window.location.search)
    return {
      client_id: params.get('client_id'),
      redirect_uri: params.get('redirect_uri'),
      scope: params.get('scope'),
      state: params.get('state'),
      code_challenge: params.get('code_challenge'),
      code_challenge_method: params.get('code_challenge_method'),
    }
  }, [])

  // Check if this is an "add account" flow
  const isAddAccountFlow = useMemo(() => {
    const params = new URLSearchParams(window.location.search)
    return params.get('add_account') === 'true'
  }, [])

  // Get the final redirect destination from OAuth state parameter
  const redirectUrl = useMemo(() => {
    // Extract the original path from the state parameter if present
    if (oauthParams.state) {
      const colonIndex = oauthParams.state.indexOf(':')
      if (colonIndex !== -1) {
        const originalPath = oauthParams.state.substring(colonIndex + 1)
        if (originalPath && originalPath !== '/') {
          return originalPath
        }
      }
    }
    return '/'
  }, [oauthParams.state])

  // Check if setup is required or user is already logged in
  useEffect(() => {
    const checkSession = async () => {
      try {
        // First, check if setup is required via health endpoint
        const healthResponse = await fetch('/api/system/health')
        if (healthResponse.ok) {
          const health = await healthResponse.json()
          if (health.setup_required) {
            // Redirect to setup page
            window.location.href = '/setup'
            return
          }
        }

        // Fetch existing accounts (if any)
        try {
          const accounts = await getAccounts()
          setExistingAccounts(accounts)

          // If this is an add_account flow, show account picker
          if (isAddAccountFlow && accounts.length > 0) {
            setShowAccountPicker(true)
            setCheckingSession(false)
            return
          }
        } catch {
          // No existing accounts, continue to login
        }

        // Try to get current user - works with session cookie
        await getCurrentUser()
        // User is authenticated, redirect to dashboard (unless adding new account)
        if (!isAddAccountFlow) {
          window.location.href = redirectUrl
          return
        }
        setCheckingSession(false)
      } catch {
        // Not authenticated, show login form
        setCheckingSession(false)
      }
    }
    checkSession()
  }, [redirectUrl, isAddAccountFlow])

  // Fetch available OAuth providers
  useEffect(() => {
    const fetchProviders = async () => {
      try {
        const providers = await oauthApi.getAvailableProviders()
        setOauthProviders(providers)
      } catch {
        // Silently fail - OAuth providers are optional
      }
    }
    fetchProviders()
  }, [])

  // Start precaching dashboard data as soon as we have existing accounts
  // This means data will be ready instantly when they switch accounts
  useEffect(() => {
    if (existingAccounts.length > 0) {
      // Start fetching dashboard data in the background
      precacheDashboard()
    }
  }, [existingAccounts])

  // Focus TOTP input when entering 2FA step
  useEffect(() => {
    if (step === '2fa_required' && totpInputRef.current) {
      totpInputRef.current.focus()
    }
  }, [step])

  const handleCredentialsSubmit = async (e: FormEvent) => {
    e.preventDefault()
    setError(null)
    setLoading(true)

    try {
      const response: SessionLoginResponse = await sessionLogin({ username, password })

      switch (response.status) {
        case 'success':
          // Session cookie is set by backend - redirect to complete OAuth flow or home
          window.location.href = redirectUrl
          break
        case '2fa_required':
          // Need to enter TOTP code
          setStep('2fa_required')
          setLoading(false)
          break
        case '2fa_setup_required':
          // User needs to set up 2FA first
          setStep('2fa_setup_required')
          setLoading(false)
          break
        default:
          throw new Error('Unexpected response')
      }
    } catch (err: unknown) {
      const error = err as { response?: { data?: { detail?: string } }, message?: string }
      setError(error.response?.data?.detail || error.message || 'Login failed')
      setLoading(false)
    }
  }

  const handleTotpSubmit = async (e: FormEvent) => {
    e.preventDefault()
    if (totpCode.length !== 6) return

    setError(null)
    setLoading(true)

    try {
      // Backend expects credentials + totp_code in single request
      const response = await verify2FA({ username, password }, totpCode)

      if (response.status === 'success') {
        // Session cookie is set by backend - redirect to complete OAuth flow or home
        window.location.href = redirectUrl
      } else {
        throw new Error('Unexpected response')
      }
    } catch (err: unknown) {
      const error = err as { response?: { data?: { detail?: string } }, message?: string }
      setError(error.response?.data?.detail || error.message || 'Verification failed')
      setTotpCode('')
      setLoading(false)
    }
  }

  const handleRecoveryCodeSubmit = async (e: FormEvent) => {
    e.preventDefault()
    if (!recoveryCode.trim()) return

    setError(null)
    setLoading(true)

    try {
      await loginWithRecoveryCode(username, password, recoveryCode.trim().toUpperCase())
      window.location.href = redirectUrl
    } catch (err: unknown) {
      const error = err as { response?: { data?: { detail?: string } }, message?: string }
      setError(error.response?.data?.detail || error.message || 'Invalid recovery code')
      setRecoveryCode('')
      setLoading(false)
    }
  }

  const handleBack = () => {
    setStep('credentials')
    setTotpCode('')
    setRecoveryCode('')
    setError(null)
    setPassword('') // Clear password for security
  }

  const handleOAuthLogin = (providerId: string) => {
    window.location.href = `/api/oauth/${providerId}/login`
  }

  const handleSwitchAccount = async (slot: number) => {
    setSwitchingAccount(true)
    try {
      await switchAccount(slot)
      // Redirect to dashboard after switching
      window.location.href = redirectUrl
    } catch (err: unknown) {
      const error = err as { response?: { data?: { detail?: string } }, message?: string }
      setError(error.response?.data?.detail || error.message || 'Failed to switch account')
      setSwitchingAccount(false)
    }
  }

  const handleAddNewAccount = () => {
    setShowAccountPicker(false)
  }

  const getProviderIcon = (providerId: string) => {
    switch (providerId) {
      case 'google':
        return (
          <svg className="w-5 h-5" viewBox="0 0 24 24">
            <path
              fill="currentColor"
              d="M22.56 12.25c0-.78-.07-1.53-.2-2.25H12v4.26h5.92c-.26 1.37-1.04 2.53-2.21 3.31v2.77h3.57c2.08-1.92 3.28-4.74 3.28-8.09z"
            />
            <path
              fill="currentColor"
              d="M12 23c2.97 0 5.46-.98 7.28-2.66l-3.57-2.77c-.98.66-2.23 1.06-3.71 1.06-2.86 0-5.29-1.93-6.16-4.53H2.18v2.84C3.99 20.53 7.7 23 12 23z"
            />
            <path
              fill="currentColor"
              d="M5.84 14.09c-.22-.66-.35-1.36-.35-2.09s.13-1.43.35-2.09V7.07H2.18C1.43 8.55 1 10.22 1 12s.43 3.45 1.18 4.93l2.85-2.22.81-.62z"
            />
            <path
              fill="currentColor"
              d="M12 5.38c1.62 0 3.06.56 4.21 1.64l3.15-3.15C17.45 2.09 14.97 1 12 1 7.7 1 3.99 3.47 2.18 7.07l3.66 2.84c.87-2.6 3.3-4.53 6.16-4.53z"
            />
          </svg>
        )
      case 'microsoft':
        return (
          <svg className="w-5 h-5" viewBox="0 0 24 24">
            <path fill="#f25022" d="M1 1h10v10H1z" />
            <path fill="#00a4ef" d="M1 13h10v10H1z" />
            <path fill="#7fba00" d="M13 1h10v10H13z" />
            <path fill="#ffb900" d="M13 13h10v10H13z" />
          </svg>
        )
      default:
        return null
    }
  }

  // Show loading while checking session
  if (checkingSession) {
    return (
      <div className="min-h-screen bg-gray-50 dark:bg-gray-900 flex items-center justify-center">
        <Loader2 size={32} className="animate-spin text-blue-500" />
      </div>
    )
  }

  // 2FA Setup Required View
  if (step === '2fa_setup_required') {
    return (
      <div className="min-h-screen bg-gray-50 dark:bg-gray-900 flex items-center justify-center px-4">
        <div className="max-w-md w-full space-y-8">
          <div className="text-center">
            <div className="flex justify-center mb-4">
              <div className="p-3 bg-amber-100 dark:bg-amber-900/30 rounded-full">
                <Shield size={32} className="text-amber-600 dark:text-amber-400" />
              </div>
            </div>
            <h2 className="text-2xl font-bold text-gray-900 dark:text-white">
              Two-Factor Authentication Required
            </h2>
            <p className="mt-2 text-sm text-gray-600 dark:text-gray-400">
              Your account role requires two-factor authentication to be enabled.
              Please set up 2FA before you can continue.
            </p>
          </div>

          <div className="bg-white dark:bg-gray-800 rounded-lg shadow-sm border border-gray-200 dark:border-gray-700 p-6">
            <p className="text-sm text-gray-600 dark:text-gray-400 mb-4">
              After setting up 2FA, you'll be able to sign in. You'll need an authenticator app like:
            </p>
            <ul className="text-sm text-gray-600 dark:text-gray-400 space-y-1 mb-6 ml-4 list-disc">
              <li>Google Authenticator</li>
              <li>Authy</li>
              <li>1Password</li>
              <li>Microsoft Authenticator</li>
            </ul>
            <p className="text-sm text-gray-600 dark:text-gray-400 mb-4">
              Please contact your administrator to set up 2FA for your account, or if you have access,
              sign in through another method and visit Account Settings.
            </p>
          </div>

          <button
            onClick={handleBack}
            className="w-full flex items-center justify-center gap-2 py-2 px-4 border border-gray-300 dark:border-gray-600 text-gray-700 dark:text-gray-300 rounded-md hover:bg-gray-50 dark:hover:bg-gray-800 transition-colors"
          >
            <ArrowLeft size={16} />
            Back to Login
          </button>
        </div>
      </div>
    )
  }

  // 2FA Code Entry View
  if (step === '2fa_required') {
    return (
      <div className="min-h-screen bg-gray-50 dark:bg-gray-900 flex items-center justify-center px-4">
        <div className="max-w-md w-full space-y-8">
          <div className="text-center">
            <div className="flex justify-center mb-4">
              <div className="p-3 bg-blue-100 dark:bg-blue-900/30 rounded-full">
                <Shield size={32} className="text-blue-600 dark:text-blue-400" />
              </div>
            </div>
            <h2 className="text-2xl font-bold text-gray-900 dark:text-white">
              Two-Factor Authentication
            </h2>
            <p className="mt-2 text-sm text-gray-600 dark:text-gray-400">
              Enter the 6-digit code from your authenticator app
            </p>
          </div>

          <form className="mt-8 space-y-6" onSubmit={handleTotpSubmit}>
            {error && (
              <div className="rounded-md bg-red-100 dark:bg-red-900/50 border border-red-300 dark:border-red-700 p-4">
                <div className="text-sm text-red-700 dark:text-red-200">{error}</div>
              </div>
            )}

            <div className="flex justify-center">
              <input
                ref={totpInputRef}
                type="text"
                inputMode="numeric"
                pattern="[0-9]*"
                autoComplete="one-time-code"
                value={totpCode}
                onChange={(e) => setTotpCode(e.target.value.replace(/\D/g, '').slice(0, 6))}
                placeholder="000000"
                maxLength={6}
                className="w-48 px-4 py-3 text-center text-2xl font-mono tracking-[0.5em] border border-gray-300 dark:border-gray-700 rounded-lg bg-white dark:bg-gray-800 text-gray-900 dark:text-white focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-transparent"
              />
            </div>

            <div className="space-y-3">
              <button
                type="submit"
                disabled={loading || totpCode.length !== 6}
                className="group relative w-full flex justify-center items-center gap-2 py-2 px-4 border border-transparent text-sm font-medium rounded-md text-white bg-blue-600 hover:bg-blue-700 focus:outline-none focus:ring-2 focus:ring-offset-2 focus:ring-blue-500 transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
              >
                {loading && <Loader2 size={16} className="animate-spin" />}
                {loading ? 'Verifying...' : 'Verify'}
              </button>

              <button
                type="button"
                onClick={() => { setStep('recovery_code'); setTotpCode(''); setError(null) }}
                className="w-full flex items-center justify-center gap-2 py-2 px-4 border border-gray-300 dark:border-gray-600 text-gray-700 dark:text-gray-300 rounded-md hover:bg-gray-50 dark:hover:bg-gray-800 transition-colors text-sm"
              >
                <Key size={14} />
                Use recovery code instead
              </button>

              <button
                type="button"
                onClick={handleBack}
                className="w-full flex items-center justify-center gap-2 py-2 px-4 border border-gray-300 dark:border-gray-600 text-gray-700 dark:text-gray-300 rounded-md hover:bg-gray-50 dark:hover:bg-gray-800 transition-colors"
              >
                <ArrowLeft size={16} />
                Back to Login
              </button>
            </div>
          </form>
        </div>
      </div>
    )
  }

  // Recovery Code View
  if (step === 'recovery_code') {
    return (
      <div className="min-h-screen bg-gray-50 dark:bg-gray-900 flex items-center justify-center px-4">
        <div className="max-w-md w-full space-y-8">
          <div className="text-center">
            <div className="flex justify-center mb-4">
              <div className="p-3 bg-amber-100 dark:bg-amber-900/30 rounded-full">
                <Key size={32} className="text-amber-600 dark:text-amber-400" />
              </div>
            </div>
            <h2 className="text-2xl font-bold text-gray-900 dark:text-white">
              Recovery Code
            </h2>
            <p className="mt-2 text-sm text-gray-600 dark:text-gray-400">
              Enter one of your saved recovery codes to sign in
            </p>
          </div>

          <form className="mt-8 space-y-6" onSubmit={handleRecoveryCodeSubmit}>
            {error && (
              <div className="rounded-md bg-red-100 dark:bg-red-900/50 border border-red-300 dark:border-red-700 p-4">
                <div className="text-sm text-red-700 dark:text-red-200">{error}</div>
              </div>
            )}

            <div className="flex justify-center">
              <input
                type="text"
                autoFocus
                autoComplete="off"
                value={recoveryCode}
                onChange={(e) => setRecoveryCode(e.target.value.toUpperCase().replace(/[^A-Z0-9]/g, '').slice(0, 10))}
                placeholder="XXXXXXXXXX"
                maxLength={10}
                className="w-48 px-4 py-3 text-center text-xl font-mono tracking-[0.3em] border border-gray-300 dark:border-gray-700 rounded-lg bg-white dark:bg-gray-800 text-gray-900 dark:text-white focus:outline-none focus:ring-2 focus:ring-amber-500 focus:border-transparent"
              />
            </div>

            <div className="space-y-3">
              <button
                type="submit"
                disabled={loading || recoveryCode.length !== 10}
                className="group relative w-full flex justify-center items-center gap-2 py-2 px-4 border border-transparent text-sm font-medium rounded-md text-white bg-amber-600 hover:bg-amber-700 focus:outline-none focus:ring-2 focus:ring-offset-2 focus:ring-amber-500 transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
              >
                {loading && <Loader2 size={16} className="animate-spin" />}
                {loading ? 'Verifying...' : 'Sign in with recovery code'}
              </button>

              <button
                type="button"
                onClick={() => { setStep('2fa_required'); setRecoveryCode(''); setError(null) }}
                className="w-full flex items-center justify-center gap-2 py-2 px-4 border border-gray-300 dark:border-gray-600 text-gray-700 dark:text-gray-300 rounded-md hover:bg-gray-50 dark:hover:bg-gray-800 transition-colors"
              >
                <ArrowLeft size={16} />
                Use authenticator app instead
              </button>
            </div>
          </form>
        </div>
      </div>
    )
  }

  // Account Picker View (when adding another account)
  if (showAccountPicker && existingAccounts.length > 0) {
    return (
      <div className="min-h-screen bg-gray-50 dark:bg-gray-900 flex items-center justify-center px-4">
        <div className="max-w-md w-full space-y-8">
          <div className="text-center">
            <div className="flex justify-center mb-4">
              <Ship size={48} className="text-blue-500 dark:text-blue-400" strokeWidth={2} />
            </div>
            <h2 className="text-3xl font-extrabold text-gray-900 dark:text-white">
              Choose an account
            </h2>
            <p className="mt-2 text-sm text-gray-500 dark:text-gray-400">
              Select an existing account or sign in with a different one
            </p>
          </div>

          {error && (
            <div className="rounded-md bg-red-100 dark:bg-red-900/50 border border-red-300 dark:border-red-700 p-4">
              <div className="text-sm text-red-700 dark:text-red-200">{error}</div>
            </div>
          )}

          <div className="space-y-2">
            {existingAccounts.map((account) => (
              <button
                type="button"
                key={account.slot}
                onClick={() => handleSwitchAccount(account.slot)}
                disabled={switchingAccount}
                className={`w-full flex items-center gap-4 p-4 rounded-lg border transition-colors ${
                  account.is_active
                    ? 'bg-blue-50 dark:bg-blue-900/30 border-blue-300 dark:border-blue-700'
                    : 'bg-white dark:bg-gray-800 border-gray-200 dark:border-gray-700 hover:bg-gray-50 dark:hover:bg-gray-700'
                } disabled:opacity-50 disabled:cursor-not-allowed`}
              >
                <div className="flex-shrink-0 w-10 h-10 bg-gray-200 dark:bg-gray-600 rounded-full flex items-center justify-center">
                  <User size={20} className="text-gray-600 dark:text-gray-300" />
                </div>
                <div className="flex-1 text-left">
                  <div className="font-medium text-gray-900 dark:text-white">
                    {account.username}
                  </div>
                  <div className="text-sm text-gray-500 dark:text-gray-400">
                    {account.email}
                  </div>
                </div>
                {account.is_active && (
                  <div className="flex-shrink-0 text-blue-500">
                    <Check size={20} />
                  </div>
                )}
              </button>
            ))}

            <div className="relative py-2">
              <div className="absolute inset-0 flex items-center">
                <div className="w-full border-t border-gray-300 dark:border-gray-600" />
              </div>
              <div className="relative flex justify-center text-sm">
                <span className="px-2 bg-gray-50 dark:bg-gray-900 text-gray-400">or</span>
              </div>
            </div>

            <button
              type="button"
              onClick={handleAddNewAccount}
              disabled={switchingAccount}
              className="w-full flex items-center justify-center gap-2 p-4 rounded-lg border border-gray-200 dark:border-gray-700 bg-white dark:bg-gray-800 hover:bg-gray-50 dark:hover:bg-gray-700 transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
            >
              <span className="text-gray-700 dark:text-gray-300">Sign in with a different account</span>
            </button>
          </div>
        </div>
      </div>
    )
  }

  // Default: Credentials Entry View
  return (
    <div className="min-h-screen bg-gray-50 dark:bg-gray-900 flex items-center justify-center px-4">
      <div className="max-w-md w-full space-y-8">
        <div className="text-center">
          <div className="flex justify-center mb-4">
            <Ship size={48} className="text-blue-500 dark:text-blue-400" strokeWidth={2} />
          </div>
          <h2 className="text-3xl font-extrabold text-gray-900 dark:text-white">
            Kubarr Dashboard
          </h2>
          <p className="mt-2 text-sm text-gray-500 dark:text-gray-400">
            Sign in to your account
          </p>
        </div>

        <form className="mt-8 space-y-6" onSubmit={handleCredentialsSubmit}>
          {error && (
            <div className="rounded-md bg-red-100 dark:bg-red-900/50 border border-red-300 dark:border-red-700 p-4">
              <div className="text-sm text-red-700 dark:text-red-200">{error}</div>
            </div>
          )}

          <div className="rounded-md shadow-sm -space-y-px">
            <div>
              <label htmlFor="username" className="sr-only">
                Username
              </label>
              <input
                id="username"
                name="username"
                type="text"
                required
                autoFocus
                value={username}
                onChange={(e) => setUsername(e.target.value)}
                className="appearance-none rounded-none relative block w-full px-3 py-2 border border-gray-300 dark:border-gray-700 placeholder-gray-500 text-gray-900 dark:text-white bg-white dark:bg-gray-800 rounded-t-md focus:outline-none focus:ring-blue-500 focus:border-blue-500 focus:z-10 sm:text-sm"
                placeholder="Username"
              />
            </div>
            <div>
              <label htmlFor="password" className="sr-only">
                Password
              </label>
              <input
                id="password"
                name="password"
                type="password"
                required
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                className="appearance-none rounded-none relative block w-full px-3 py-2 border border-gray-300 dark:border-gray-700 placeholder-gray-500 text-gray-900 dark:text-white bg-white dark:bg-gray-800 rounded-b-md focus:outline-none focus:ring-blue-500 focus:border-blue-500 focus:z-10 sm:text-sm"
                placeholder="Password"
              />
            </div>
          </div>

          <div>
            <button
              type="submit"
              disabled={loading}
              className="group relative w-full flex justify-center items-center gap-2 py-2 px-4 border border-transparent text-sm font-medium rounded-md text-white bg-blue-600 hover:bg-blue-700 focus:outline-none focus:ring-2 focus:ring-offset-2 focus:ring-blue-500 transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
            >
              {loading && <Loader2 size={16} className="animate-spin" />}
              {loading ? 'Signing in...' : 'Sign in'}
            </button>
          </div>

          {oauthProviders.length > 0 && (
            <>
              <div className="relative">
                <div className="absolute inset-0 flex items-center">
                  <div className="w-full border-t border-gray-600" />
                </div>
                <div className="relative flex justify-center text-sm">
                  <span className="px-2 bg-gray-900 text-gray-400">Or continue with</span>
                </div>
              </div>

              <div className="space-y-2">
                {oauthProviders.map((provider) => (
                  <button
                    key={provider.id}
                    type="button"
                    onClick={() => handleOAuthLogin(provider.id)}
                    className="w-full flex items-center justify-center gap-3 py-2 px-4 border border-gray-600 rounded-md bg-gray-800 hover:bg-gray-700 text-gray-200 transition-colors"
                  >
                    {getProviderIcon(provider.id)}
                    <span>Sign in with {provider.name}</span>
                  </button>
                ))}
              </div>
            </>
          )}
        </form>
      </div>
    </div>
  )
}
