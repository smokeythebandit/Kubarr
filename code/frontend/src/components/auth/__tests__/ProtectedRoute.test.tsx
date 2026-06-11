import { describe, it, expect, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import ProtectedRoute from '../ProtectedRoute';

// Mock useAuth with controllable return values
const mockUseAuth = vi.fn();
vi.mock('../../../contexts/AuthContext', () => ({
  useAuth: () => mockUseAuth(),
}));

describe('ProtectedRoute', () => {
  it('renders children when user is authenticated', () => {
    mockUseAuth.mockReturnValue({
      isAuthenticated: true,
      loading: false,
    });

    render(
      <ProtectedRoute>
        <div data-testid="protected-content">Secret content</div>
      </ProtectedRoute>,
    );

    expect(screen.getByTestId('protected-content')).toBeInTheDocument();
  });

  it('redirects to /login when user is not authenticated', () => {
    mockUseAuth.mockReturnValue({
      isAuthenticated: false,
      loading: false,
    });

    const { container } = render(
      <ProtectedRoute>
        <div data-testid="protected-content">Secret content</div>
      </ProtectedRoute>,
    );

    expect(window.location.href).toBe('/login');
    expect(container.innerHTML).toBe('');
  });
});
