import React, { createContext, useContext, useState, useEffect, ReactNode, useMemo } from 'react';
import { getCurrentUser, User } from '../api/users';
import { getAccounts, switchAccount as apiSwitchAccount, AccountInfo } from '../api/auth';

interface AuthContextType {
  user: User | null;
  loading: boolean;
  isAuthenticated: boolean;
  isAdmin: boolean;
  permissions: Set<string>;
  allowedApps: Set<string> | null;
  hasPermission: (permission: string) => boolean;
  canAccessApp: (appName: string) => boolean;
  checkAuth: () => Promise<void>;
  logout: () => void;
  // Multi-account support
  accounts: AccountInfo[];
  otherAccounts: AccountInfo[];
  switchAccount: (slot: number) => Promise<void>;
  refreshAccounts: () => Promise<void>;
}

const AuthContext = createContext<AuthContextType | undefined>(undefined);

export const useAuth = () => {
  const context = useContext(AuthContext);
  if (!context) {
    throw new Error('useAuth must be used within an AuthProvider');
  }
  return context;
};

interface AuthProviderProps {
  children: ReactNode;
}

export const AuthProvider: React.FC<AuthProviderProps> = ({ children }) => {
  const [user, setUser] = useState<User | null>(null);
  const [loading, setLoading] = useState(true);
  const [accounts, setAccounts] = useState<AccountInfo[]>([]);

  const refreshAccounts = async () => {
    try {
      const accountList = await getAccounts();
      setAccounts(accountList);
    } catch {
      // Silently fail - accounts list is optional
    }
  };

  const checkAuth = async () => {
    try {
      setLoading(true);
      const currentUser = await getCurrentUser();
      setUser(currentUser);
      // Refresh the accounts list from the backend
      if (currentUser) {
        await refreshAccounts();
      }
    } catch (error) {
      setUser(null);
      // If we get a 401, the user is not authenticated
      // OAuth2-Proxy should redirect to login automatically
      console.error('Authentication check failed:', error);
    } finally {
      setLoading(false);
    }
  };

  const logout = () => {
    setUser(null);
    // Clear token from localStorage
    localStorage.removeItem('access_token');
  };

  const switchAccount = async (slot: number) => {
    try {
      await apiSwitchAccount(slot);
      // Reload the page to refresh with the new session
      window.location.reload();
    } catch (error) {
      console.error('Failed to switch account:', error);
      throw error;
    }
  };

  useEffect(() => {
    // Skip auth check on login and setup pages to prevent redirect loop
    // These pages don't require authentication
    if (window.location.pathname === '/login' || window.location.pathname === '/setup') {
      setLoading(false);
      return;
    }
    checkAuth();
  }, []);

  // Check if user has admin role
  const isAdmin = user?.roles?.some(r => r.name === 'admin') || false;

  // Get user permissions from the backend response
  const permissions = useMemo(() => {
    if (!user?.permissions) return new Set<string>();
    return new Set(user.permissions);
  }, [user]);

  const allowedApps = useMemo(() => {
    if (!user) return new Set<string>();
    if (isAdmin) return null; // null means all apps
    return new Set(user.allowed_apps || []);
  }, [user, isAdmin]);

  const hasPermission = (permission: string): boolean => {
    if (isAdmin) return true;
    return permissions.has(permission);
  };

  const canAccessApp = (appName: string): boolean => {
    if (isAdmin) return true;
    if (allowedApps === null) return true;
    return allowedApps.has(appName);
  };

  // Get accounts other than the current active one
  const otherAccounts = useMemo(() => {
    return accounts.filter(a => !a.is_active);
  }, [accounts]);

  const value: AuthContextType = {
    user,
    loading,
    isAuthenticated: user !== null,
    isAdmin,
    permissions,
    allowedApps,
    hasPermission,
    canAccessApp,
    checkAuth,
    logout,
    accounts,
    otherAccounts,
    switchAccount,
    refreshAccounts,
  };

  return <AuthContext.Provider value={value}>{children}</AuthContext.Provider>;
};
