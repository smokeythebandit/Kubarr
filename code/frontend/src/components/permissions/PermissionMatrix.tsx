import React, { useState, useEffect, useCallback } from 'react';
import { Role, PermissionInfo, getPermissions, getRoles, setRolePermissions } from '../../api/roles';
import { appsApi } from '../../api/apps';
import type { AppConfig } from '../../types';

interface PermissionsByCategory {
  [category: string]: PermissionInfo[];
}

interface AppInfo {
  [appName: string]: AppConfig;
}

const PermissionMatrix: React.FC = () => {
  const [roles, setRoles] = useState<Role[]>([]);
  const [permissionsByCategory, setPermissionsByCategory] = useState<PermissionsByCategory>({});
  const [rolePermissions, setRolePermissionsState] = useState<{ [roleId: number]: Set<string> }>({});
  const [_installedApps, setInstalledApps] = useState<Set<string>>(new Set());
  const [appInfo, setAppInfo] = useState<AppInfo>({});
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [successMessage, setSuccessMessage] = useState<string | null>(null);
  const [pendingChanges, setPendingChanges] = useState<{ [roleId: number]: Set<string> }>({});

  const loadData = useCallback(async () => {
    try {
      setLoading(true);
      setError(null);

      const [rolesData, permissionsData, installedData, catalogData] = await Promise.all([
        getRoles(),
        getPermissions(),
        appsApi.getInstalled(),
        appsApi.getCatalog(),
      ]);

      setRoles(rolesData);
      setInstalledApps(new Set(installedData));

      // Create app info lookup
      const appLookup: AppInfo = {};
      for (const app of catalogData) {
        appLookup[app.name] = app;
      }
      setAppInfo(appLookup);

      // Group permissions by category, filtering app permissions to only installed apps
      const grouped: PermissionsByCategory = {};
      const installedSet = new Set(installedData);

      for (const perm of permissionsData) {
        // For App Access category, only include permissions for installed apps
        if (perm.category === 'App Access') {
          // Extract app name from permission key (e.g., "app.sonarr" -> "sonarr")
          const appName = perm.key.replace('app.', '');
          if (!installedSet.has(appName)) {
            continue; // Skip permissions for apps that aren't installed
          }
        }

        if (!grouped[perm.category]) {
          grouped[perm.category] = [];
        }
        grouped[perm.category].push(perm);
      }
      setPermissionsByCategory(grouped);

      // Initialize role permissions from the roles data
      const permMap: { [roleId: number]: Set<string> } = {};
      for (const role of rolesData) {
        permMap[role.id] = new Set(role.permissions || []);
      }
      setRolePermissionsState(permMap);
      setPendingChanges({});

    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to load permission data');
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    loadData();
  }, [loadData]);

  const isAdminRole = (role: Role): boolean => {
    return role.is_system && role.name === 'admin';
  };

  const hasPermission = (roleId: number, permission: string): boolean => {
    const role = roles.find(r => r.id === roleId);
    if (role && isAdminRole(role)) {
      return true; // Admin always has all permissions
    }

    // Check pending changes first
    if (pendingChanges[roleId]) {
      return pendingChanges[roleId].has(permission);
    }

    return rolePermissions[roleId]?.has(permission) || false;
  };

  const hasPendingChanges = (roleId: number): boolean => {
    if (!pendingChanges[roleId]) return false;
    const currentPerms = rolePermissions[roleId] || new Set<string>();
    const pendingPerms = pendingChanges[roleId];

    if (currentPerms.size !== pendingPerms.size) return true;
    for (const perm of currentPerms) {
      if (!pendingPerms.has(perm)) return true;
    }
    return false;
  };

  const togglePermission = (roleId: number, permission: string) => {
    const role = roles.find(r => r.id === roleId);
    if (role && isAdminRole(role)) {
      return; // Cannot modify admin role
    }

    setPendingChanges(prev => {
      const currentSet = prev[roleId] || new Set(rolePermissions[roleId] || []);
      const newSet = new Set(currentSet);

      if (newSet.has(permission)) {
        newSet.delete(permission);
      } else {
        newSet.add(permission);
      }

      return { ...prev, [roleId]: newSet };
    });
  };

  const saveRolePermissions = async (roleId: number) => {
    const role = roles.find(r => r.id === roleId);
    if (!role || isAdminRole(role)) return;

    const permissionsToSave = pendingChanges[roleId];
    if (!permissionsToSave) return;

    try {
      setSaving(roleId);
      setError(null);

      await setRolePermissions(roleId, { permissions: Array.from(permissionsToSave) });

      // Update local state
      setRolePermissionsState(prev => ({
        ...prev,
        [roleId]: permissionsToSave,
      }));

      // Clear pending changes for this role
      setPendingChanges(prev => {
        const newPending = { ...prev };
        delete newPending[roleId];
        return newPending;
      });

      setSuccessMessage(`Permissions saved for ${role.name}`);
      setTimeout(() => setSuccessMessage(null), 3000);

    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to save permissions');
    } finally {
      setSaving(null);
    }
  };

  const discardChanges = (roleId: number) => {
    setPendingChanges(prev => {
      const newPending = { ...prev };
      delete newPending[roleId];
      return newPending;
    });
  };

  const getCategoryIcon = (category: string): string => {
    switch (category) {
      case 'Apps': return '📦';
      case 'App Access': return '🌐';
      case 'Storage': return '💾';
      case 'Logs': return '📋';
      case 'Monitoring': return '📊';
      case 'Users': return '👥';
      case 'Roles': return '🔑';
      case 'Settings': return '⚙️';
      default: return '📄';
    }
  };

  const getAppIconUrl = (permissionKey: string): string | null => {
    if (!permissionKey.startsWith('app.')) return null;
    const appName = permissionKey.replace('app.', '');
    const app = appInfo[appName];
    if (app?.icon) {
      return `/api/apps/catalog/${appName}/icon`;
    }
    return null;
  };

  const getAppDisplayName = (permissionKey: string): string => {
    if (!permissionKey.startsWith('app.')) return permissionKey;
    const appName = permissionKey.replace('app.', '');
    const app = appInfo[appName];
    return app?.display_name || appName;
  };

  if (loading) {
    return (
      <div className="flex items-center justify-center py-12">
        <div className="animate-spin rounded-full h-8 w-8 border-b-2 border-blue-600"></div>
        <span className="ml-3 text-gray-600 dark:text-gray-400">Loading permissions...</span>
      </div>
    );
  }

  const categories = Object.keys(permissionsByCategory).sort();

  return (
    <div className="space-y-6">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div>
          <h3 className="text-lg font-semibold text-gray-900 dark:text-white">Permission Matrix</h3>
          <p className="mt-1 text-sm text-gray-500 dark:text-gray-400">
            Configure which permissions are granted to each role. Admin role always has all permissions.
          </p>
        </div>
        <button
          onClick={loadData}
          className="px-4 py-2 bg-gray-100 dark:bg-gray-700 text-gray-700 dark:text-gray-300 rounded-lg hover:bg-gray-200 dark:hover:bg-gray-600 transition-colors"
        >
          Refresh
        </button>
      </div>

      {/* Messages */}
      {error && (
        <div className="p-4 bg-red-50 dark:bg-red-900/30 border border-red-200 dark:border-red-800 rounded-lg text-red-700 dark:text-red-300">
          {error}
        </div>
      )}

      {successMessage && (
        <div className="p-4 bg-green-50 dark:bg-green-900/30 border border-green-200 dark:border-green-800 rounded-lg text-green-700 dark:text-green-300">
          {successMessage}
        </div>
      )}

      {/* Matrix Table */}
      <div className="overflow-x-auto bg-white dark:bg-gray-800 rounded-lg border border-gray-200 dark:border-gray-700">
        <table className="min-w-full">
          <thead className="bg-gray-50 dark:bg-gray-700">
            <tr>
              <th className="sticky left-0 bg-gray-50 dark:bg-gray-700 px-6 py-3 text-left text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wider border-r border-gray-200 dark:border-gray-600 min-w-[200px]">
                Permission
              </th>
              {roles.map(role => (
                <th key={role.id} className="px-4 py-3 text-center min-w-[120px]">
                  <div className="flex flex-col items-center gap-1">
                    <span className={`text-xs font-medium uppercase tracking-wider ${
                      isAdminRole(role)
                        ? 'text-blue-600 dark:text-blue-400'
                        : 'text-gray-500 dark:text-gray-400'
                    }`}>
                      {role.name}
                    </span>
                    {isAdminRole(role) && (
                      <span className="text-xs text-gray-400 dark:text-gray-500 font-normal normal-case">
                        (all access)
                      </span>
                    )}
                    {hasPendingChanges(role.id) && (
                      <div className="flex gap-1 mt-1">
                        <button
                          onClick={() => saveRolePermissions(role.id)}
                          disabled={saving === role.id}
                          className="px-2 py-0.5 text-xs bg-green-600 hover:bg-green-700 text-white rounded transition-colors disabled:opacity-50"
                        >
                          {saving === role.id ? 'Saving...' : 'Save'}
                        </button>
                        <button
                          onClick={() => discardChanges(role.id)}
                          disabled={saving === role.id}
                          className="px-2 py-0.5 text-xs bg-gray-500 hover:bg-gray-600 text-white rounded transition-colors disabled:opacity-50"
                        >
                          Discard
                        </button>
                      </div>
                    )}
                  </div>
                </th>
              ))}
            </tr>
          </thead>
          <tbody className="divide-y divide-gray-200 dark:divide-gray-700">
            {categories.map(category => (
              <React.Fragment key={category}>
                {/* Category Header Row */}
                <tr className="bg-gray-100 dark:bg-gray-700/70">
                  <td
                    colSpan={roles.length + 1}
                    className="sticky left-0 px-6 py-2 text-sm font-semibold text-gray-700 dark:text-gray-300 bg-gray-100 dark:bg-gray-700/70"
                  >
                    <span className="mr-2">{getCategoryIcon(category)}</span>
                    {category}
                  </td>
                </tr>
                {/* Permission Rows */}
                {permissionsByCategory[category].map(permission => {
                  const isAppPermission = permission.category === 'App Access';
                  const appIconUrl = isAppPermission ? getAppIconUrl(permission.key) : null;
                  const displayName = isAppPermission ? getAppDisplayName(permission.key) : permission.key;

                  return (
                    <tr key={permission.key} className="hover:bg-gray-50 dark:hover:bg-gray-700/50 transition-colors">
                      <td className="sticky left-0 bg-white dark:bg-gray-800 px-6 py-3 border-r border-gray-200 dark:border-gray-600">
                        <div className="flex items-center gap-3">
                          {appIconUrl && (
                            <img
                              src={appIconUrl}
                              alt=""
                              className="w-6 h-6 rounded"
                              onError={(e) => {
                                (e.target as HTMLImageElement).style.display = 'none';
                              }}
                            />
                          )}
                          <div className="flex flex-col">
                            <span className="text-sm font-medium text-gray-900 dark:text-white">
                              {displayName}
                            </span>
                            <span className="text-xs text-gray-500 dark:text-gray-400">
                              {permission.description}
                            </span>
                          </div>
                        </div>
                      </td>
                      {roles.map(role => (
                        <td key={role.id} className="px-4 py-3 text-center">
                          <label className="inline-flex items-center justify-center cursor-pointer">
                            <input
                              type="checkbox"
                              checked={hasPermission(role.id, permission.key)}
                              onChange={() => togglePermission(role.id, permission.key)}
                              disabled={isAdminRole(role)}
                              className={`w-5 h-5 rounded border-gray-300 dark:border-gray-600 text-blue-600 focus:ring-blue-500 focus:ring-offset-0 transition-colors ${
                                isAdminRole(role)
                                  ? 'opacity-50 cursor-not-allowed bg-blue-100 dark:bg-blue-900'
                                  : 'cursor-pointer hover:border-blue-500'
                              }`}
                            />
                          </label>
                        </td>
                      ))}
                    </tr>
                  );
                })}
              </React.Fragment>
            ))}
          </tbody>
        </table>
      </div>

      {/* Legend */}
      <div className="flex items-center gap-6 text-sm text-gray-500 dark:text-gray-400">
        <div className="flex items-center gap-2">
          <input type="checkbox" checked readOnly className="w-4 h-4 rounded text-blue-600" />
          <span>Permission granted</span>
        </div>
        <div className="flex items-center gap-2">
          <input type="checkbox" checked={false} readOnly className="w-4 h-4 rounded" />
          <span>Permission denied</span>
        </div>
        <div className="flex items-center gap-2">
          <span className="w-4 h-4 rounded bg-gray-200 dark:bg-gray-600"></span>
          <span>Admin role (always has all permissions)</span>
        </div>
      </div>
    </div>
  );
};

export default PermissionMatrix;
