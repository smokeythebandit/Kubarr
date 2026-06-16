import UserList from '../../users/UserList';
import type { User } from '../../../api/users';
import { SettingsTabLayout } from './SettingsTabLayout';

type PendingUsersTabProps = {
  pendingUsers: User[];
  onApproveUser: (user: User) => void;
  onRejectUser: (user: User) => void;
  onDeleteUser: (user: User) => void;
};

export function PendingUsersTab({ pendingUsers, onApproveUser, onRejectUser, onDeleteUser }: PendingUsersTabProps) {
  return (
    <SettingsTabLayout
      title="Pending Approval"
      description="Users waiting for approval to access the system."
    >
      <UserList
        users={pendingUsers}
        onApprove={onApproveUser}
        onReject={onRejectUser}
        onDelete={onDeleteUser}
        showActions={true}
      />
    </SettingsTabLayout>
  );
}
