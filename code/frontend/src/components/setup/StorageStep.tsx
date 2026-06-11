import React, { useEffect, useMemo, useState } from 'react';
import { AlertCircle, Check, Database, Loader2, Network, Server } from 'lucide-react';
import { setupApi, StorageConfigRequest, StorageMode, StorageValidationResponse } from '../../api/setup';

interface StorageStepProps {
  stage: 'configure' | 'provision';
  initialRequest?: StorageConfigRequest | null;
  validation?: StorageValidationResponse | null;
  onValidated: (request: StorageConfigRequest, validation: StorageValidationResponse) => void;
  onProvisioned: () => void;
  onBack?: () => void;
}

const modeOptions: Array<{
  mode: StorageMode;
  label: string;
  description: string;
  icon: React.ReactNode;
}> = [
  {
    mode: 'external_nfs',
    label: 'External NFS',
    description: 'Existing NFS share',
    icon: <Network className="w-5 h-5" />,
  },
  {
    mode: 'managed_nfs',
    label: 'Managed NFS',
    description: 'Cluster-backed share',
    icon: <Server className="w-5 h-5" />,
  },
];

const StorageStep: React.FC<StorageStepProps> = ({
  stage,
  initialRequest,
  validation,
  onValidated,
  onProvisioned,
  onBack,
}) => {
  const [mode, setMode] = useState<StorageMode>(initialRequest?.mode || 'external_nfs');
  const [nfsServer, setNfsServer] = useState(initialRequest?.external_nfs?.server || '');
  const [nfsExportPath, setNfsExportPath] = useState(initialRequest?.external_nfs?.export_path || '');
  const [storageClass, setStorageClass] = useState(initialRequest?.managed_nfs?.storage_class || '');
  const [managedSize, setManagedSize] = useState(initialRequest?.managed_nfs?.size || '1Ti');
  const [error, setError] = useState<string | null>(null);
  const [working, setWorking] = useState(false);

  useEffect(() => {
    if (!initialRequest) return;
    setMode(initialRequest.mode);
    setNfsServer(initialRequest.external_nfs?.server || '');
    setNfsExportPath(initialRequest.external_nfs?.export_path || '');
    setStorageClass(initialRequest.managed_nfs?.storage_class || '');
    setManagedSize(initialRequest.managed_nfs?.size || '1Ti');
  }, [initialRequest]);

  const request = useMemo<StorageConfigRequest>(() => ({
    mode,
    external_nfs: mode === 'external_nfs'
      ? { server: nfsServer.trim(), export_path: nfsExportPath.trim() }
      : null,
    managed_nfs: mode === 'managed_nfs'
      ? { storage_class: storageClass.trim() || null, size: managedSize.trim() || '1Ti' }
      : null,
  }), [managedSize, mode, nfsExportPath, nfsServer, storageClass]);

  const validateForm = () => {
    if (mode === 'external_nfs' && (!nfsServer.trim() || !nfsExportPath.trim())) {
      return 'NFS server and export path are required';
    }
    if (mode === 'managed_nfs' && !managedSize.trim()) {
      return 'Managed NFS size is required';
    }
    return null;
  };

  const handleValidate = async () => {
    const formError = validateForm();
    if (formError) {
      setError(formError);
      return;
    }

    setWorking(true);
    setError(null);
    try {
      const result = await setupApi.validateStorage(request);
      if (!result.valid) {
        setError(result.message || 'Storage validation failed');
        return;
      }
      await setupApi.configureStorage(request);
      onValidated(request, result);
    } catch (err: any) {
      setError(err.response?.data?.detail || 'Storage validation failed');
    } finally {
      setWorking(false);
    }
  };

  const handleProvision = async () => {
    setWorking(true);
    setError(null);
    try {
      await setupApi.provisionStorage();
      onProvisioned();
    } catch (err: any) {
      setError(err.response?.data?.detail || 'Storage provisioning failed');
    } finally {
      setWorking(false);
    }
  };

  if (stage === 'provision') {
    return (
      <div className="space-y-6">
        <div>
          <h2 className="text-xl font-semibold text-gray-900 dark:text-white mb-2">
            Storage Validation
          </h2>
          <p className="text-gray-500 dark:text-gray-400 text-sm">
            Media storage will be mounted as <span className="font-mono">/data</span>.
          </p>
        </div>

        {error && <ErrorBox message={error} />}

        <div className="space-y-3">
          {(validation?.checks || []).map((check) => (
            <div
              key={check.name}
              className="flex items-center justify-between rounded-md border border-gray-200 dark:border-gray-700 bg-gray-50 dark:bg-gray-900 px-4 py-3"
            >
              <div>
                <div className="text-sm font-medium text-gray-900 dark:text-white">{check.message}</div>
                {check.warning && <div className="text-xs text-yellow-500 mt-1">Warning only</div>}
              </div>
              {check.warning ? (
                <AlertCircle className="w-5 h-5 text-yellow-500" />
              ) : (
                <Check className="w-5 h-5 text-green-500" />
              )}
            </div>
          ))}
        </div>

        {validation?.warnings?.length ? (
          <div className="rounded-md bg-yellow-900/20 border border-yellow-700 p-4 text-sm text-yellow-200">
            {validation.warnings.join(' ')}
          </div>
        ) : null}

        <div className="flex justify-between">
          <button
            type="button"
            onClick={onBack}
            disabled={working}
            className="px-6 py-2 border border-gray-600 text-gray-300 font-medium rounded-md hover:bg-gray-700 focus:outline-none focus:ring-2 focus:ring-gray-500 disabled:opacity-50"
          >
            Back
          </button>
          <button
            type="button"
            onClick={handleProvision}
            disabled={working || !validation?.valid}
            className="px-6 py-2 bg-blue-600 text-white font-medium rounded-md hover:bg-blue-700 focus:outline-none focus:ring-2 focus:ring-blue-500 disabled:opacity-50"
          >
            {working ? (
              <span className="flex items-center space-x-2">
                <Loader2 className="w-4 h-4 animate-spin" />
                <span>Provisioning...</span>
              </span>
            ) : (
              'Provision Storage'
            )}
          </button>
        </div>
      </div>
    );
  }

  return (
    <div className="space-y-6">
      <div>
        <h2 className="text-xl font-semibold text-gray-900 dark:text-white mb-2">
          Storage
        </h2>
        <p className="text-gray-500 dark:text-gray-400 text-sm">
          Choose the shared media backend for downloads and libraries.
        </p>
      </div>

      {error && <ErrorBox message={error} />}

      <div className="grid gap-3 sm:grid-cols-2">
        {modeOptions.map((option) => (
          <button
            key={option.mode}
            type="button"
            onClick={() => setMode(option.mode)}
            className={`text-left rounded-md border p-4 transition ${
              mode === option.mode
                ? 'border-blue-500 bg-blue-50 dark:bg-blue-900/20'
                : 'border-gray-200 dark:border-gray-700 bg-gray-50 dark:bg-gray-900 hover:border-gray-400'
            }`}
          >
            <div className="flex items-center space-x-2 text-gray-900 dark:text-white">
              {option.icon}
              <span className="font-medium">{option.label}</span>
            </div>
            <div className="mt-2 text-xs text-gray-500 dark:text-gray-400">{option.description}</div>
          </button>
        ))}
      </div>

      <div className="space-y-4">
        {mode === 'external_nfs' && (
          <>
            <Input label="NFS Server" value={nfsServer} onChange={setNfsServer} placeholder="192.168.1.10" />
            <Input label="Export Path" value={nfsExportPath} onChange={setNfsExportPath} placeholder="/tank/media" />
          </>
        )}

        {mode === 'managed_nfs' && (
          <>
            <Input label="StorageClass" value={storageClass} onChange={setStorageClass} placeholder="Default StorageClass" />
            <Input label="Size" value={managedSize} onChange={setManagedSize} placeholder="1Ti" />
            <div className="rounded-md bg-yellow-900/20 border border-yellow-700 p-4 text-sm text-yellow-200">
              Managed NFS is intended for simple single-node installs.
            </div>
          </>
        )}

        <div className="rounded-md bg-gray-100 dark:bg-gray-900 p-4">
          <div className="flex items-center space-x-2 text-sm font-medium text-gray-700 dark:text-gray-300">
            <Database className="w-4 h-4" />
            <span>Container Mount</span>
          </div>
          <div className="mt-2 text-sm font-mono text-gray-900 dark:text-white">/data</div>
        </div>
      </div>

      <div className="flex justify-end">
        <button
          type="button"
          onClick={handleValidate}
          disabled={working}
          className="px-6 py-2 bg-blue-600 text-white font-medium rounded-md hover:bg-blue-700 focus:outline-none focus:ring-2 focus:ring-blue-500 disabled:opacity-50"
        >
          {working ? (
            <span className="flex items-center space-x-2">
              <Loader2 className="w-4 h-4 animate-spin" />
              <span>Validating...</span>
            </span>
          ) : (
            'Validate Storage'
          )}
        </button>
      </div>
    </div>
  );
};

const ErrorBox: React.FC<{ message: string }> = ({ message }) => (
  <div className="rounded-md bg-red-900/50 border border-red-700 p-4">
    <div className="flex items-center space-x-2">
      <AlertCircle className="w-5 h-5 text-red-400" />
      <span className="text-sm text-red-200">{message}</span>
    </div>
  </div>
);

const Input: React.FC<{
  label: string;
  value: string;
  onChange: (value: string) => void;
  placeholder?: string;
  type?: string;
}> = ({ label, value, onChange, placeholder, type = 'text' }) => (
  <div>
    <label className="block text-sm font-medium text-gray-700 dark:text-gray-300">
      {label}
    </label>
    <input
      type={type}
      value={value}
      onChange={(event) => onChange(event.target.value)}
      placeholder={placeholder}
      className="mt-1 block w-full px-3 py-2 border border-gray-300 dark:border-gray-700 bg-white dark:bg-gray-900 text-gray-900 dark:text-white rounded-md focus:outline-none focus:ring-blue-500 focus:border-blue-500 sm:text-sm"
    />
  </div>
);

export default StorageStep;
