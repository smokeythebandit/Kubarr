import { useLocation } from 'react-router-dom'

interface PageTransitionProps {
  children: React.ReactNode
  className?: string
}

export function PageTransition({ children, className = '' }: PageTransitionProps) {
  const location = useLocation()

  return (
    <div key={location.pathname} className={`page-transition ${className}`}>
      {children}
    </div>
  )
}
