import React from 'react';
import { useAuth } from '../../contexts/AuthContext';

interface AdminRouteProps {
  children: React.ReactNode;
}

/**
 * AdminRoute component ensures user is authenticated AND has admin privileges
 * before allowing access to the route.
 */
const AdminRoute: React.FC<AdminRouteProps> = ({ children }) => {
  const { isAuthenticated, isAdmin, loading } = useAuth();

  if (loading) {
    return (
      <div className="d-flex justify-content-center align-items-center" style={{ minHeight: '400px' }}>
        <div className="spinner-border" role="status">
          <span className="visually-hidden">Loading...</span>
        </div>
      </div>
    );
  }

  if (!isAuthenticated) {
    // Redirect to login page
    window.location.href = '/login';
    return null;
  }

  if (!isAdmin) {
    return (
      <div className="container mt-4">
        <div className="alert alert-danger">
          You do not have permission to access this page. Admin privileges required.
        </div>
      </div>
    );
  }

  return <>{children}</>;
};

export default AdminRoute;
