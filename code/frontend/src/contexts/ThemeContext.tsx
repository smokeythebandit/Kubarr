import React, { createContext, useContext, useState, useEffect, ReactNode, useCallback } from 'react';
import { Theme, updateMyPreferences } from '../api/users';
import { useAuth } from './AuthContext';

type ResolvedTheme = 'light' | 'dark';

interface ThemeContextType {
  theme: Theme;
  resolvedTheme: ResolvedTheme;
  setTheme: (theme: Theme) => void;
}

const ThemeContext = createContext<ThemeContextType | undefined>(undefined);

const THEME_STORAGE_KEY = 'kubarr-theme';

function getSystemTheme(): ResolvedTheme {
  if (typeof window !== 'undefined' && window.matchMedia) {
    return window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
  }
  return 'light';
}

function resolveTheme(theme: Theme): ResolvedTheme {
  if (theme === 'system') {
    return getSystemTheme();
  }
  return theme;
}

export const useTheme = () => {
  const context = useContext(ThemeContext);
  if (!context) {
    throw new Error('useTheme must be used within a ThemeProvider');
  }
  return context;
};

interface ThemeProviderProps {
  children: ReactNode;
}

export const ThemeProvider: React.FC<ThemeProviderProps> = ({ children }) => {
  const { user, isAuthenticated } = useAuth();

  // Initialize theme from localStorage or default to 'system'
  const [theme, setThemeState] = useState<Theme>(() => {
    const stored = localStorage.getItem(THEME_STORAGE_KEY);
    if (stored && ['system', 'light', 'dark'].includes(stored)) {
      return stored as Theme;
    }
    return 'system';
  });

  const [resolvedTheme, setResolvedTheme] = useState<ResolvedTheme>(() => resolveTheme(theme));

  // Update resolved theme when theme or system preference changes
  useEffect(() => {
    setResolvedTheme(resolveTheme(theme));

    // Listen for system theme changes when using 'system' theme
    if (theme === 'system') {
      const mediaQuery = window.matchMedia('(prefers-color-scheme: dark)');
      const handleChange = () => {
        setResolvedTheme(getSystemTheme());
      };

      mediaQuery.addEventListener('change', handleChange);
      return () => mediaQuery.removeEventListener('change', handleChange);
    }
  }, [theme]);

  // Apply dark class to document
  useEffect(() => {
    if (resolvedTheme === 'dark') {
      document.documentElement.classList.add('dark');
    } else {
      document.documentElement.classList.remove('dark');
    }
  }, [resolvedTheme]);

  // Sync theme from user preferences when authenticated
  useEffect(() => {
    if (isAuthenticated && user?.preferences?.theme) {
      const userTheme = user.preferences.theme;
      if (['system', 'light', 'dark'].includes(userTheme)) {
        setThemeState(userTheme);
        localStorage.setItem(THEME_STORAGE_KEY, userTheme);
      }
    }
  }, [isAuthenticated, user?.preferences?.theme]);

  const setTheme = useCallback(async (newTheme: Theme) => {
    // Update local state immediately
    setThemeState(newTheme);
    localStorage.setItem(THEME_STORAGE_KEY, newTheme);

    // Sync to backend if authenticated
    if (isAuthenticated) {
      try {
        await updateMyPreferences({ theme: newTheme });
      } catch (error) {
        console.error('Failed to save theme preference:', error);
      }
    }
  }, [isAuthenticated]);

  const value: ThemeContextType = {
    theme,
    resolvedTheme,
    setTheme,
  };

  return <ThemeContext.Provider value={value}>{children}</ThemeContext.Provider>;
};
