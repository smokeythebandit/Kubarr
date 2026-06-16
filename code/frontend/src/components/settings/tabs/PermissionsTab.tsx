import PermissionMatrix from '../../permissions/PermissionMatrix';
import { SettingsTabLayout } from './SettingsTabLayout';

export function PermissionsTab() {
  return (
    <SettingsTabLayout
      title="Permissions"
      description="Configure fine-grained access control for each role, including which applications users can access."
    >
      <PermissionMatrix />
    </SettingsTabLayout>
  );
}
