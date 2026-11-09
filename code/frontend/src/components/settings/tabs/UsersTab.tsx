import UserList from '../../users/UserList';
import type { User } from '../../../api/users';
import { SettingsTabLayout } from './SettingsTabLayout';

type UsersTabProps = {
  users: User[];
  onCreateUser: () => void;
  onEditUser: (user: User) => void;
  onDeleteUser: (user: User) => void;
  onApproveUser: (user: User) => void;
  /** users.manage: create, edit, approve and delete. */
  canManage: boolean;
};

export function UsersTab({ users, onCreateUser, onEditUser, onDeleteUser, onApproveUser, canManage }: UsersTabProps) {
  return (
    <SettingsTabLayout
      title="All Users"
      description="Manage user accounts and access."
      actions={canManage && (
        <button
          className="px-4 py-2 bg-blue-600 hover:bg-blue-700 rounded-md font-medium transition-colors"
          onClick={onCreateUser}
        >
          Create New User
        </button>
      )}
    >
      <UserList
        users={users}
        onEdit={onEditUser}
        onDelete={onDeleteUser}
        onApprove={onApproveUser}
        showActions={canManage}
      />
    </SettingsTabLayout>
  );
}
