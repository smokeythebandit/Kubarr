import type { ReactNode } from 'react';

type SettingsTabLayoutProps = {
  title: string;
  description: string;
  actions?: ReactNode;
  children: ReactNode;
};

type SettingsCardProps = {
  children: ReactNode;
  className?: string;
};

type SettingsEmptyStateProps = {
  children: ReactNode;
};

export function SettingsTabLayout({ title, description, actions, children }: SettingsTabLayoutProps) {
  return (
    <div className="space-y-6">
      <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
        <div>
          <h3 className="text-2xl font-bold text-gray-900 dark:text-white">{title}</h3>
          <p className="text-sm text-gray-500 dark:text-gray-400 mt-1">{description}</p>
        </div>
        {actions && <div className="flex flex-wrap gap-2 sm:justify-end">{actions}</div>}
      </div>
      {children}
    </div>
  );
}

export function SettingsCard({ children, className = '' }: SettingsCardProps) {
  return (
    <div className={`bg-white dark:bg-gray-800 rounded-lg border border-gray-200 dark:border-gray-700 p-6 ${className}`}>
      {children}
    </div>
  );
}

export function SettingsEmptyState({ children }: SettingsEmptyStateProps) {
  return (
    <SettingsCard>
      <div className="text-gray-500 dark:text-gray-400 text-sm py-8 text-center">
        {children}
      </div>
    </SettingsCard>
  );
}
