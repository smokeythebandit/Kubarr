import React, { useState, useEffect } from 'react';
import { useNavigate } from 'react-router-dom';
import {
  setupApi,
  SetupRequest,
  StorageConfigRequest,
  StorageValidationResponse,
} from '../api/setup';
import BootstrapStep from '../components/setup/BootstrapStep';
import ServerStep from '../components/setup/ServerStep';
import StorageStep from '../components/setup/StorageStep';

type SetupStep = 'storage' | 'storageProvision' | 'bootstrap' | 'server' | 'admin' | 'summary';

const SetupPage: React.FC = () => {
  const navigate = useNavigate();
  const [currentStep, setCurrentStep] = useState<SetupStep>('storage');
  const [error, setError] = useState('');
  const [loading, setLoading] = useState(false);
  const [initialLoading, setInitialLoading] = useState(true);

  // Form data
  const [serverConfig, setServerConfig] = useState<{ name: string } | null>(null);
  const [storageRequest, setStorageRequest] = useState<StorageConfigRequest | null>(null);
  const [storageValidation, setStorageValidation] = useState<StorageValidationResponse | null>(null);
  const [adminUsername, setAdminUsername] = useState('admin');
  const [adminEmail, setAdminEmail] = useState('');
  const [adminPassword, setAdminPassword] = useState('');
  const [confirmPassword, setConfirmPassword] = useState('');

  // Check setup status and determine starting step
  useEffect(() => {
    const checkSetup = async () => {
      try {
        const { setup_required } = await setupApi.checkRequired();
        if (!setup_required) {
          navigate('/');
          return;
        }

        // Get detailed status to determine which step to show
        const status = await setupApi.getStatus();

        const storageConfig = await setupApi.getStorageConfig().catch(() => null);
        if (storageConfig) {
          const cfg = storageConfig.config_json || {};
          const textField = (key: string) => (typeof cfg[key] === 'string' ? cfg[key] as string : '');
          setStorageRequest({
            mode: storageConfig.mode,
            external_nfs: storageConfig.mode === 'external_nfs'
              ? {
                  server: textField('server'),
                  export_path: textField('export_path'),
                }
              : null,
            managed_nfs: storageConfig.mode === 'managed_nfs'
              ? {
                  storage_class: textField('storage_class') || null,
                  size: textField('size') || '1Ti',
                }
              : null,
          });
        }
        if (storageConfig?.validation_json && 'checks' in storageConfig.validation_json) {
          setStorageValidation(storageConfig.validation_json as StorageValidationResponse);
        }

        if (status.bootstrap_complete && status.server_configured) {
          setCurrentStep('admin');
          // Load server config
          const config = await setupApi.getServerConfig();
          if (config) {
            setServerConfig({ name: config.name });
          }
        } else if (status.bootstrap_complete) {
          setCurrentStep('server');
        } else if (status.storage_configured) {
          setCurrentStep('bootstrap');
        } else if (storageConfig?.validated) {
          setCurrentStep('bootstrap');
        } else {
          setCurrentStep('storage');
        }
      } catch (err) {
        // If we can't check, start from beginning
        setCurrentStep('storage');
      } finally {
        setInitialLoading(false);
      }
    };
    checkSetup();
  }, [navigate]);

  const validateAdminForm = (): boolean => {
    if (!adminUsername.trim()) {
      setError('Username is required');
      return false;
    }
    if (!adminEmail.trim() || !adminEmail.includes('@')) {
      setError('Valid email is required');
      return false;
    }
    if (!adminPassword || adminPassword.length < 8) {
      setError('Password must be at least 8 characters');
      return false;
    }
    if (adminPassword !== confirmPassword) {
      setError('Passwords do not match');
      return false;
    }
    setError('');
    return true;
  };

  const handleAdminNext = () => {
    if (validateAdminForm()) {
      setCurrentStep('summary');
    }
  };

  const handleSubmit = async () => {
    setError('');
    setLoading(true);

    try {
      const setupData: SetupRequest = {
        admin_username: adminUsername,
        admin_email: adminEmail,
        admin_password: adminPassword,
      };

      const result = await setupApi.initialize(setupData);

      if (result.success) {
        // Force full page reload to trigger auth flow
        window.location.href = '/';
      } else {
        setError(result.message || 'Setup failed');
      }
    } catch (err: any) {
      setError(err.response?.data?.detail || 'Setup failed. Please try again.');
    } finally {
      setLoading(false);
    }
  };

  const steps = [
    { id: 'storage', label: 'Storage', number: 1 },
    { id: 'storageProvision', label: 'Validate', number: 2 },
    { id: 'bootstrap', label: 'System', number: 3 },
    { id: 'server', label: 'Server', number: 4 },
    { id: 'admin', label: 'Admin User', number: 5 },
    { id: 'summary', label: 'Summary', number: 6 },
  ];

  const currentStepIndex = steps.findIndex((s) => s.id === currentStep);

  if (initialLoading) {
    return (
      <div className="min-h-screen bg-gray-50 dark:bg-gray-900 flex items-center justify-center">
        <div className="text-gray-500 dark:text-gray-400">Loading...</div>
      </div>
    );
  }

  return (
    <div className="min-h-screen bg-gray-50 dark:bg-gray-900 flex items-center justify-center px-4 py-12">
      <div className="max-w-2xl w-full space-y-8">
        {/* Header */}
        <div className="text-center">
          <h1 className="text-3xl font-extrabold text-gray-900 dark:text-white">
            Welcome to Kubarr
          </h1>
          <p className="mt-2 text-gray-500 dark:text-gray-400">
            Let's set up your media management dashboard
          </p>
        </div>

        {/* Progress Indicator */}
        <div className="flex justify-center items-center space-x-4">
          {steps.map((step, index) => (
            <React.Fragment key={step.id}>
              <div className="flex items-center">
                <div
                  className={`w-8 h-8 rounded-full flex items-center justify-center text-sm font-medium ${
                    index <= currentStepIndex
                      ? 'bg-blue-600 text-white'
                      : 'bg-gray-200 dark:bg-gray-700 text-gray-500 dark:text-gray-400'
                  }`}
                >
                  {step.number}
                </div>
                <span
                  className={`ml-2 text-sm hidden sm:inline ${
                    index <= currentStepIndex ? 'text-white' : 'text-gray-500'
                  }`}
                >
                  {step.label}
                </span>
              </div>
              {index < steps.length - 1 && (
                <div
                  className={`w-8 sm:w-16 h-0.5 ${
                    index < currentStepIndex ? 'bg-blue-600' : 'bg-gray-700'
                  }`}
                />
              )}
            </React.Fragment>
          ))}
        </div>

        {/* Form Container */}
        <div className="bg-white dark:bg-gray-800 rounded-lg shadow-xl p-8">
          {error && (
            <div className="mb-6 rounded-md bg-red-900 p-4">
              <div className="text-sm text-red-200">{error}</div>
            </div>
          )}

          {/* Step 1: Storage */}
          {currentStep === 'storage' && (
            <StorageStep
              stage="configure"
              initialRequest={storageRequest}
              validation={storageValidation}
              onValidated={(request, validation) => {
                setStorageRequest(request);
                setStorageValidation(validation);
                setCurrentStep('storageProvision');
              }}
              onProvisioned={() => setCurrentStep('bootstrap')}
            />
          )}

          {/* Step 2: Storage validation/provisioning */}
          {currentStep === 'storageProvision' && (
            <StorageStep
              stage="provision"
              initialRequest={storageRequest}
              validation={storageValidation}
              onValidated={(request, validation) => {
                setStorageRequest(request);
                setStorageValidation(validation);
              }}
              onProvisioned={() => setCurrentStep('bootstrap')}
              onBack={() => setCurrentStep('storage')}
            />
          )}

          {/* Step 3: Bootstrap */}
          {currentStep === 'bootstrap' && (
            <BootstrapStep onComplete={() => setCurrentStep('server')} />
          )}

          {/* Step 4: Server Configuration */}
          {currentStep === 'server' && (
            <ServerStep
              onComplete={(config) => {
                setServerConfig(config);
                setCurrentStep('admin');
              }}
              onBack={() => setCurrentStep('bootstrap')}
              initialConfig={serverConfig ? { name: serverConfig.name, storage_path: '/data' } : null}
            />
          )}

          {/* Step 5: Admin User */}
          {currentStep === 'admin' && (
            <div className="space-y-6">
              <div>
                <h2 className="text-xl font-semibold text-gray-900 dark:text-white mb-2">
                  Admin User
                </h2>
                <p className="text-gray-500 dark:text-gray-400 text-sm">
                  Create the initial administrator account.
                </p>
              </div>

              <div className="space-y-4">
                <div>
                  <label
                    htmlFor="adminUsername"
                    className="block text-sm font-medium text-gray-700 dark:text-gray-300"
                  >
                    Username
                  </label>
                  <input
                    type="text"
                    id="adminUsername"
                    value={adminUsername}
                    onChange={(e) => setAdminUsername(e.target.value)}
                    className="mt-1 block w-full px-3 py-2 border border-gray-300 dark:border-gray-700 bg-white dark:bg-gray-900 text-gray-900 dark:text-white rounded-md focus:outline-none focus:ring-blue-500 focus:border-blue-500 sm:text-sm"
                  />
                </div>

                <div>
                  <label
                    htmlFor="adminEmail"
                    className="block text-sm font-medium text-gray-700 dark:text-gray-300"
                  >
                    Email
                  </label>
                  <input
                    type="email"
                    id="adminEmail"
                    value={adminEmail}
                    onChange={(e) => setAdminEmail(e.target.value)}
                    className="mt-1 block w-full px-3 py-2 border border-gray-300 dark:border-gray-700 bg-white dark:bg-gray-900 text-gray-900 dark:text-white rounded-md focus:outline-none focus:ring-blue-500 focus:border-blue-500 sm:text-sm"
                    placeholder="admin@example.com"
                  />
                </div>

                <div>
                  <label
                    htmlFor="adminPassword"
                    className="block text-sm font-medium text-gray-700 dark:text-gray-300"
                  >
                    Password
                  </label>
                  <input
                    type="password"
                    id="adminPassword"
                    value={adminPassword}
                    onChange={(e) => setAdminPassword(e.target.value)}
                    className="mt-1 block w-full px-3 py-2 border border-gray-300 dark:border-gray-700 bg-white dark:bg-gray-900 text-gray-900 dark:text-white rounded-md focus:outline-none focus:ring-blue-500 focus:border-blue-500 sm:text-sm"
                    placeholder="Minimum 8 characters"
                  />
                </div>

                <div>
                  <label
                    htmlFor="confirmPassword"
                    className="block text-sm font-medium text-gray-700 dark:text-gray-300"
                  >
                    Confirm Password
                  </label>
                  <input
                    type="password"
                    id="confirmPassword"
                    value={confirmPassword}
                    onChange={(e) => setConfirmPassword(e.target.value)}
                    className="mt-1 block w-full px-3 py-2 border border-gray-300 dark:border-gray-700 bg-white dark:bg-gray-900 text-gray-900 dark:text-white rounded-md focus:outline-none focus:ring-blue-500 focus:border-blue-500 sm:text-sm"
                  />
                </div>
              </div>

              <div className="flex justify-between">
                <button
                  type="button"
                  onClick={() => setCurrentStep('server')}
                  className="px-6 py-2 border border-gray-600 text-gray-300 font-medium rounded-md hover:bg-gray-700 focus:outline-none focus:ring-2 focus:ring-gray-500"
                >
                  Back
                </button>
                <button
                  type="button"
                  onClick={handleAdminNext}
                  className="px-6 py-2 bg-blue-600 text-white font-medium rounded-md hover:bg-blue-700 focus:outline-none focus:ring-2 focus:ring-blue-500"
                >
                  Next
                </button>
              </div>
            </div>
          )}

          {/* Step 6: Summary */}
          {currentStep === 'summary' && (
            <div className="space-y-6">
              <div>
                <h2 className="text-xl font-semibold text-gray-900 dark:text-white mb-2">
                  Review & Confirm
                </h2>
                <p className="text-gray-500 dark:text-gray-400 text-sm">
                  Please review your configuration before completing setup.
                </p>
              </div>

              <div className="space-y-4">
                {/* System Components */}
                <div className="bg-gray-100 dark:bg-gray-900 rounded-md p-4">
                  <h3 className="text-sm font-medium text-gray-700 dark:text-gray-300 mb-3">
                    System Components
                  </h3>
                  <div className="flex items-center space-x-2 text-sm text-green-400">
                    <svg className="w-5 h-5" fill="currentColor" viewBox="0 0 20 20">
                      <path
                        fillRule="evenodd"
                        d="M10 18a8 8 0 100-16 8 8 0 000 16zm3.707-9.293a1 1 0 00-1.414-1.414L9 10.586 7.707 9.293a1 1 0 00-1.414 1.414l2 2a1 1 0 001.414 0l4-4z"
                        clipRule="evenodd"
                      />
                    </svg>
                    <span>All system components installed</span>
                  </div>
                </div>

                {/* Storage */}
                <div className="bg-gray-100 dark:bg-gray-900 rounded-md p-4">
                  <h3 className="text-sm font-medium text-gray-700 dark:text-gray-300 mb-3">
                    Storage
                  </h3>
                  <dl className="space-y-2">
                    <div className="flex justify-between">
                      <dt className="text-sm text-gray-500">Mode:</dt>
                      <dd className="text-sm text-gray-900 dark:text-white">
                        {storageRequest?.mode?.replace('_', ' ') || 'Configured'}
                      </dd>
                    </div>
                    <div className="flex justify-between">
                      <dt className="text-sm text-gray-500">Mount:</dt>
                      <dd className="text-sm text-gray-900 dark:text-white font-mono">/data</dd>
                    </div>
                  </dl>
                </div>

                {/* Server Configuration */}
                <div className="bg-gray-100 dark:bg-gray-900 rounded-md p-4">
                  <h3 className="text-sm font-medium text-gray-700 dark:text-gray-300 mb-3">
                    Server Configuration
                  </h3>
                  <dl className="space-y-2">
                    <div className="flex justify-between">
                      <dt className="text-sm text-gray-500">Server Name:</dt>
                      <dd className="text-sm text-gray-900 dark:text-white">
                        {serverConfig?.name || 'Kubarr'}
                      </dd>
                    </div>
                  </dl>
                </div>

                {/* Admin User */}
                <div className="bg-gray-100 dark:bg-gray-900 rounded-md p-4">
                  <h3 className="text-sm font-medium text-gray-700 dark:text-gray-300 mb-3">
                    Admin User
                  </h3>
                  <dl className="space-y-2">
                    <div className="flex justify-between">
                      <dt className="text-sm text-gray-500">Username:</dt>
                      <dd className="text-sm text-gray-900 dark:text-white">{adminUsername}</dd>
                    </div>
                    <div className="flex justify-between">
                      <dt className="text-sm text-gray-500">Email:</dt>
                      <dd className="text-sm text-gray-900 dark:text-white">{adminEmail}</dd>
                    </div>
                  </dl>
                </div>
              </div>

              <div className="flex justify-between">
                <button
                  type="button"
                  onClick={() => setCurrentStep('admin')}
                  disabled={loading}
                  className="px-6 py-2 border border-gray-600 text-gray-300 font-medium rounded-md hover:bg-gray-700 focus:outline-none focus:ring-2 focus:ring-gray-500 disabled:opacity-50"
                >
                  Back
                </button>
                <button
                  type="button"
                  onClick={handleSubmit}
                  disabled={loading}
                  className="px-6 py-2 bg-green-600 text-white font-medium rounded-md hover:bg-green-700 focus:outline-none focus:ring-2 focus:ring-green-500 disabled:opacity-50"
                >
                  {loading ? 'Setting up...' : 'Complete Setup'}
                </button>
              </div>
            </div>
          )}
        </div>
      </div>
    </div>
  );
};

export default SetupPage;
