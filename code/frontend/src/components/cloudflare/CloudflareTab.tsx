import { useState, useEffect, useRef } from 'react';
import {
  Cloud,
  Eye,
  EyeOff,
  ExternalLink,
  RefreshCw,
  AlertCircle,
  CheckCircle,
  ChevronRight,
} from 'lucide-react';
import {
  cloudflareApi,
  CloudflareTunnelConfig,
  CloudflareTunnelStatus,
  TunnelStatus,
  ValidateTokenResponse,
  ZoneInfo,
} from '../../api/cloudflare';

// ============================================================================
// Status badge
// ============================================================================

function StatusBadge({ status }: { status: TunnelStatus }) {
  const configs: Record<TunnelStatus, { label: string; classes: string }> = {
    not_deployed: {
      label: 'Not Deployed',
      classes: 'bg-gray-100 dark:bg-gray-700 text-gray-600 dark:text-gray-300',
    },
    deploying: {
      label: 'Deploying',
      classes: 'bg-blue-100 dark:bg-blue-900 text-blue-700 dark:text-blue-300',
    },
    running: {
      label: 'Running',
      classes: 'bg-green-100 dark:bg-green-900 text-green-700 dark:text-green-300',
    },
    failed: {
      label: 'Failed',
      classes: 'bg-red-100 dark:bg-red-900 text-red-700 dark:text-red-300',
    },
    removing: {
      label: 'Removing',
      classes: 'bg-yellow-100 dark:bg-yellow-900 text-yellow-700 dark:text-yellow-300',
    },
  };

  const { label, classes } = configs[status] ?? configs.not_deployed;
  return (
    <span
      className={`inline-flex items-center px-2.5 py-0.5 rounded text-xs font-medium ${classes}`}
    >
      {(status === 'deploying' || status === 'removing') && (
        <RefreshCw size={10} className="mr-1 animate-spin" />
      )}
      {label}
    </span>
  );
}

// ============================================================================
// Wizard step type
// ============================================================================

type WizardStep = 'idle' | 'validating' | 'validated';

// ============================================================================
// Main component
// ============================================================================

export function CloudflareTab() {
  // ── Config / status from server ──────────────────────────────────────────
  const [config, setConfig] = useState<CloudflareTunnelConfig | null>(null);
  const [liveStatus, setLiveStatus] = useState<CloudflareTunnelStatus | null>(null);
  const [loading, setLoading] = useState(true);
  const [removing, setRemoving] = useState(false);
  const [showRemoveConfirm, setShowRemoveConfirm] = useState(false);
  const [apiError, setApiError] = useState<string | null>(null);

  // ── Wizard state (only relevant when config === null) ─────────────────────
  const [wizardStep, setWizardStep] = useState<WizardStep>('idle');
  const [apiToken, setApiToken] = useState('');
  const [showApiToken, setShowApiToken] = useState(false);
  const [validationData, setValidationData] = useState<ValidateTokenResponse | null>(null);
  const [selectedZone, setSelectedZone] = useState<ZoneInfo | null>(null);
  const [subdomain, setSubdomain] = useState('');
  const [tunnelName, setTunnelName] = useState('');
  const [deploying, setDeploying] = useState(false);
  const [wizardError, setWizardError] = useState<string | null>(null);

  const pollRef = useRef<ReturnType<typeof setInterval> | null>(null);

  // ── Data loading ──────────────────────────────────────────────────────────

  const fetchConfig = async () => {
    try {
      const data = await cloudflareApi.getConfig();
      setConfig(data);
    } catch (err) {
      setApiError(err instanceof Error ? err.message : 'Failed to load Cloudflare config');
    }
  };

  const fetchStatus = async () => {
    try {
      const status = await cloudflareApi.getStatus();
      setLiveStatus(status);
    } catch {
      // silently ignore status poll failures
    }
  };

  useEffect(() => {
    (async () => {
      setLoading(true);
      await Promise.all([fetchConfig(), fetchStatus()]);
      setLoading(false);
    })();
  }, []);

  // ── Status polling when deploying / removing ──────────────────────────────

  const currentStatus: TunnelStatus = config
    ? (liveStatus?.status ?? config.status)
    : 'not_deployed';

  useEffect(() => {
    if (currentStatus === 'deploying' || currentStatus === 'removing') {
      if (!pollRef.current) {
        pollRef.current = setInterval(fetchStatus, 5000);
      }
    } else {
      if (pollRef.current) {
        clearInterval(pollRef.current);
        pollRef.current = null;
      }
    }
    return () => {
      if (pollRef.current) {
        clearInterval(pollRef.current);
        pollRef.current = null;
      }
    };
  }, [currentStatus]);

  // ── Wizard actions ────────────────────────────────────────────────────────

  const handleValidate = async () => {
    if (!apiToken.trim()) {
      setWizardError('Please enter your Cloudflare API token');
      return;
    }
    setWizardError(null);
    setWizardStep('validating');
    try {
      const data = await cloudflareApi.validateToken({ api_token: apiToken.trim() });
      setValidationData(data);
      setSelectedZone(data.zones[0] ?? null);
      setWizardStep('validated');
    } catch (err) {
      setWizardStep('idle');
      setWizardError(
        err instanceof Error ? err.message : 'Failed to validate Cloudflare API token',
      );
    }
  };

  const handleDeploy = async () => {
    if (!tunnelName.trim()) {
      setWizardError('Please enter a tunnel name');
      return;
    }
    if (!selectedZone) {
      setWizardError('Please select a zone');
      return;
    }
    if (!subdomain.trim()) {
      setWizardError('Please enter a subdomain');
      return;
    }
    setWizardError(null);
    setDeploying(true);
    try {
      const result = await cloudflareApi.saveConfig({
        name: tunnelName.trim(),
        api_token: apiToken.trim(),
        account_id: validationData!.account_id,
        zone_id: selectedZone.id,
        zone_name: selectedZone.name,
        subdomain: subdomain.trim(),
      });
      setConfig(result);
      // Reset wizard state
      setApiToken('');
      setWizardStep('idle');
      setValidationData(null);
      setSelectedZone(null);
      setSubdomain('');
      setTunnelName('');
      await fetchStatus();
    } catch (err) {
      setWizardError(
        err instanceof Error ? err.message : 'Failed to deploy Cloudflare Tunnel',
      );
    } finally {
      setDeploying(false);
    }
  };

  const handleResetWizard = () => {
    setWizardStep('idle');
    setWizardError(null);
    setValidationData(null);
    setSelectedZone(null);
    setSubdomain('');
    setTunnelName('');
  };

  // ── Remove tunnel ─────────────────────────────────────────────────────────

  const handleRemove = async () => {
    setApiError(null);
    setRemoving(true);
    setShowRemoveConfirm(false);
    try {
      await cloudflareApi.deleteConfig();
      setConfig(null);
      setLiveStatus(null);
    } catch (err) {
      setApiError(err instanceof Error ? err.message : 'Failed to remove Cloudflare Tunnel');
    } finally {
      setRemoving(false);
    }
  };

  // ── Render ────────────────────────────────────────────────────────────────

  if (loading) {
    return (
      <div className="flex items-center justify-center py-12">
        <div className="animate-spin rounded-full h-8 w-8 border-b-2 border-blue-500" />
      </div>
    );
  }

  const isDeployed = config !== null;
  const isBusy =
    removing || currentStatus === 'deploying' || currentStatus === 'removing';

  return (
    <div className="space-y-6">
      {/* ── Header ── */}
      <div className="flex items-center justify-between">
        <div>
          <div className="flex items-center space-x-3">
            <Cloud size={24} className="text-orange-500" />
            <h3 className="text-2xl font-bold text-gray-900 dark:text-white">
              Cloudflare Tunnel
            </h3>
            {isDeployed && <StatusBadge status={currentStatus} />}
          </div>
          <p className="text-gray-500 dark:text-gray-400 text-sm mt-1">
            Expose Kubarr to the internet securely without port-forwarding.
          </p>
        </div>
      </div>

      {/* ── Global API error ── */}
      {apiError && (
        <div className="flex items-start space-x-2 bg-red-50 dark:bg-red-900/30 border border-red-200 dark:border-red-800 rounded-lg p-4">
          <AlertCircle size={18} className="text-red-500 flex-shrink-0 mt-0.5" />
          <p className="text-sm text-red-700 dark:text-red-400">{apiError}</p>
        </div>
      )}

      {isDeployed ? (
        /* ================================================================
           Configured view — tunnel exists
        ================================================================ */
        <>
          {/* Deployment error banner */}
          {config.error && (
            <div className="flex items-start space-x-2 bg-red-50 dark:bg-red-900/30 border border-red-200 dark:border-red-800 rounded-lg p-4">
              <AlertCircle size={18} className="text-red-500 flex-shrink-0 mt-0.5" />
              <div>
                <p className="text-sm font-medium text-red-800 dark:text-red-300">
                  Deployment error
                </p>
                <p className="text-sm text-red-700 dark:text-red-400 mt-0.5">{config.error}</p>
              </div>
            </div>
          )}

          {/* Tunnel summary card */}
          <div className="bg-white dark:bg-gray-800 rounded-lg border border-gray-200 dark:border-gray-700 p-6 space-y-4">
            <h4 className="text-base font-semibold text-gray-900 dark:text-white">
              Tunnel Configuration
            </h4>

            <div className="space-y-2 text-sm">
              <div className="flex items-center gap-2 text-gray-700 dark:text-gray-300">
                <span className="font-medium text-gray-500 dark:text-gray-400 w-24">Name</span>
                <span>{config.name}</span>
              </div>

              {config.hostname && (
                <div className="flex items-center gap-2 text-gray-700 dark:text-gray-300">
                  <span className="font-medium text-gray-500 dark:text-gray-400 w-24">
                    Hostname
                  </span>
                  <a
                    href={`https://${config.hostname}`}
                    target="_blank"
                    rel="noopener noreferrer"
                    className="text-blue-600 dark:text-blue-400 hover:underline inline-flex items-center gap-1"
                    data-testid="hostname-link"
                  >
                    {config.hostname}
                    <ExternalLink size={12} />
                  </a>
                </div>
              )}

              {config.zone_name && (
                <div className="flex items-center gap-2 text-gray-700 dark:text-gray-300">
                  <span className="font-medium text-gray-500 dark:text-gray-400 w-24">Zone</span>
                  <span>{config.zone_name}</span>
                </div>
              )}
            </div>

            {/* Pod status */}
            {liveStatus && liveStatus.total_pods > 0 && (
              <div className="pt-2 border-t border-gray-100 dark:border-gray-700">
                <p className="text-sm text-gray-600 dark:text-gray-300">
                  Pods ready:{' '}
                  <span className="font-medium">
                    {liveStatus.ready_pods}/{liveStatus.total_pods}
                  </span>
                </p>
              </div>
            )}
          </div>

          {/* Remove section */}
          <div className="bg-white dark:bg-gray-800 rounded-lg border border-red-200 dark:border-red-800 p-6 space-y-3">
            <h4 className="text-base font-semibold text-gray-900 dark:text-white">
              Remove Tunnel
            </h4>
            <p className="text-sm text-gray-500 dark:text-gray-400">
              Permanently removes the tunnel from Cloudflare, deletes the DNS record, and
              uninstalls cloudflared from the cluster.
            </p>

            {!showRemoveConfirm ? (
              <button
                onClick={() => setShowRemoveConfirm(true)}
                disabled={isBusy}
                className="px-4 py-2 bg-red-600 hover:bg-red-700 disabled:opacity-50 text-white text-sm font-medium rounded-md transition-colors flex items-center gap-2"
                aria-label="Remove Tunnel"
              >
                {removing && <RefreshCw size={14} className="animate-spin" />}
                Remove Tunnel
              </button>
            ) : (
              <div className="flex items-center gap-3">
                <button
                  onClick={handleRemove}
                  disabled={isBusy}
                  className="px-4 py-2 bg-red-600 hover:bg-red-700 disabled:opacity-50 text-white text-sm font-medium rounded-md transition-colors"
                >
                  Yes, remove it
                </button>
                <button
                  onClick={() => setShowRemoveConfirm(false)}
                  className="px-4 py-2 bg-gray-100 dark:bg-gray-700 hover:bg-gray-200 dark:hover:bg-gray-600 text-gray-700 dark:text-gray-300 text-sm font-medium rounded-md transition-colors"
                >
                  Cancel
                </button>
              </div>
            )}
          </div>
        </>
      ) : (
        /* ================================================================
           Wizard — no tunnel configured yet
        ================================================================ */
        <div className="bg-white dark:bg-gray-800 rounded-lg border border-gray-200 dark:border-gray-700 p-6 space-y-5">
          <h4 className="text-base font-semibold text-gray-900 dark:text-white">
            Connect Cloudflare Account
          </h4>

          {/* Wizard error */}
          {wizardError && (
            <div className="flex items-start space-x-2 bg-red-50 dark:bg-red-900/30 border border-red-200 dark:border-red-800 rounded-lg p-3">
              <AlertCircle size={16} className="text-red-500 flex-shrink-0 mt-0.5" />
              <p className="text-sm text-red-700 dark:text-red-400">{wizardError}</p>
            </div>
          )}

          {/* ── Step 1: API token input ── */}
          {(wizardStep === 'idle' || wizardStep === 'validating') && (
            <div className="space-y-4">
              <div>
                <label
                  htmlFor="cf-api-token"
                  className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1"
                >
                  Cloudflare API Token
                </label>
                <div className="relative">
                  <input
                    id="cf-api-token"
                    type={showApiToken ? 'text' : 'password'}
                    value={apiToken}
                    onChange={(e) => setApiToken(e.target.value)}
                    onKeyDown={(e) => e.key === 'Enter' && wizardStep === 'idle' && handleValidate()}
                    placeholder="Paste your Cloudflare API token"
                    disabled={wizardStep === 'validating'}
                    aria-label="Cloudflare API Token"
                    className="w-full px-3 py-2 pr-10 border border-gray-300 dark:border-gray-600 rounded-md bg-white dark:bg-gray-700 text-gray-900 dark:text-white placeholder-gray-400 focus:outline-none focus:ring-2 focus:ring-blue-500 disabled:opacity-50"
                  />
                  <button
                    type="button"
                    onClick={() => setShowApiToken((v) => !v)}
                    aria-label={showApiToken ? 'Hide token' : 'Show token'}
                    className="absolute right-2 top-1/2 -translate-y-1/2 text-gray-400 hover:text-gray-600 dark:hover:text-gray-200"
                  >
                    {showApiToken ? <EyeOff size={16} /> : <Eye size={16} />}
                  </button>
                </div>
                <p className="text-xs text-gray-500 dark:text-gray-400 mt-1">
                  Create a token at{' '}
                  <a
                    href="https://dash.cloudflare.com/profile/api-tokens"
                    target="_blank"
                    rel="noopener noreferrer"
                    className="text-blue-600 dark:text-blue-400 hover:underline inline-flex items-center gap-0.5"
                  >
                    Cloudflare API Tokens
                    <ExternalLink size={10} />
                  </a>{' '}
                  with <strong>Zone:Read</strong>, <strong>DNS:Edit</strong>, and{' '}
                  <strong>Cloudflare Tunnel:Edit</strong> permissions.
                </p>
              </div>

              <button
                onClick={handleValidate}
                disabled={wizardStep === 'validating'}
                aria-label="Connect Cloudflare Account"
                className="px-4 py-2 bg-blue-600 hover:bg-blue-700 disabled:opacity-50 text-white text-sm font-medium rounded-md transition-colors flex items-center gap-2"
              >
                {wizardStep === 'validating' ? (
                  <>
                    <RefreshCw size={14} className="animate-spin" />
                    Validating…
                  </>
                ) : (
                  <>
                    Connect Cloudflare Account
                    <ChevronRight size={14} />
                  </>
                )}
              </button>
            </div>
          )}

          {/* ── Step 2: Zone + subdomain + tunnel name ── */}
          {wizardStep === 'validated' && validationData && (
            <div className="space-y-4">
              {/* Success banner */}
              <div className="flex items-center gap-2 bg-green-50 dark:bg-green-900/30 border border-green-200 dark:border-green-700 rounded-lg p-3">
                <CheckCircle size={16} className="text-green-500 flex-shrink-0" />
                <p className="text-sm text-green-700 dark:text-green-300">
                  Connected — {validationData.zones.length} zone
                  {validationData.zones.length !== 1 ? 's' : ''} found
                </p>
              </div>

              {/* Zone dropdown */}
              <div>
                <label
                  htmlFor="cf-zone"
                  className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1"
                >
                  Zone (domain)
                </label>
                <select
                  id="cf-zone"
                  value={selectedZone?.id ?? ''}
                  onChange={(e) => {
                    const zone = validationData.zones.find((z) => z.id === e.target.value);
                    setSelectedZone(zone ?? null);
                  }}
                  disabled={deploying}
                  aria-label="Select zone"
                  className="w-full px-3 py-2 border border-gray-300 dark:border-gray-600 rounded-md bg-white dark:bg-gray-700 text-gray-900 dark:text-white focus:outline-none focus:ring-2 focus:ring-blue-500 disabled:opacity-50"
                >
                  {validationData.zones.map((z) => (
                    <option key={z.id} value={z.id}>
                      {z.name}
                    </option>
                  ))}
                  {validationData.zones.length === 0 && (
                    <option value="" disabled>
                      No active zones found
                    </option>
                  )}
                </select>
              </div>

              {/* Subdomain */}
              <div>
                <label
                  htmlFor="cf-subdomain"
                  className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1"
                >
                  Subdomain
                </label>
                <div className="flex items-center gap-2">
                  <input
                    id="cf-subdomain"
                    type="text"
                    value={subdomain}
                    onChange={(e) => setSubdomain(e.target.value)}
                    placeholder="e.g. kubarr"
                    disabled={deploying}
                    aria-label="Subdomain"
                    className="flex-1 px-3 py-2 border border-gray-300 dark:border-gray-600 rounded-md bg-white dark:bg-gray-700 text-gray-900 dark:text-white placeholder-gray-400 focus:outline-none focus:ring-2 focus:ring-blue-500 disabled:opacity-50"
                  />
                  {selectedZone && subdomain && (
                    <span className="text-sm text-gray-500 dark:text-gray-400 whitespace-nowrap">
                      → {subdomain}.{selectedZone.name}
                    </span>
                  )}
                </div>
              </div>

              {/* Tunnel name */}
              <div>
                <label
                  htmlFor="cf-tunnel-name"
                  className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1"
                >
                  Tunnel Name
                </label>
                <input
                  id="cf-tunnel-name"
                  type="text"
                  value={tunnelName}
                  onChange={(e) => setTunnelName(e.target.value)}
                  placeholder="e.g. Home Kubarr (tunnel name)"
                  disabled={deploying}
                  aria-label="Tunnel name"
                  className="w-full px-3 py-2 border border-gray-300 dark:border-gray-600 rounded-md bg-white dark:bg-gray-700 text-gray-900 dark:text-white placeholder-gray-400 focus:outline-none focus:ring-2 focus:ring-blue-500 disabled:opacity-50"
                />
              </div>

              {/* Actions */}
              <div className="flex items-center gap-3 pt-1">
                <button
                  onClick={handleDeploy}
                  disabled={deploying || validationData.zones.length === 0}
                  aria-label="Deploy Tunnel"
                  className="px-4 py-2 bg-blue-600 hover:bg-blue-700 disabled:opacity-50 text-white text-sm font-medium rounded-md transition-colors flex items-center gap-2"
                >
                  {deploying && <RefreshCw size={14} className="animate-spin" />}
                  Deploy Tunnel
                </button>
                <button
                  type="button"
                  onClick={handleResetWizard}
                  disabled={deploying}
                  className="text-sm text-blue-600 dark:text-blue-400 hover:underline disabled:opacity-50"
                >
                  Use different account
                </button>
              </div>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
