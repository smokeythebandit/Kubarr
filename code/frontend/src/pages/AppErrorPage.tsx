import { useSearchParams, Link } from 'react-router-dom'
import { Home, ArrowLeft, AlertCircle, WifiOff, ServerOff, RefreshCw } from 'lucide-react'

export default function AppErrorPage() {
  const [searchParams] = useSearchParams()
  const appName = searchParams.get('app') || 'Unknown'
  const reason = searchParams.get('reason') || 'unknown'
  const details = searchParams.get('details') || ''

  const getErrorInfo = () => {
    switch (reason) {
      case 'connection_failed':
        return {
          icon: WifiOff,
          title: 'Connection Failed',
          description: `Unable to connect to ${appName}. The app might be starting up or experiencing issues.`,
        }
      case 'not_found':
        return {
          icon: ServerOff,
          title: 'App Not Found',
          description: `The app "${appName}" is not installed or not ready yet.`,
        }
      default:
        return {
          icon: AlertCircle,
          title: 'App Error',
          description: `Something went wrong while connecting to ${appName}.`,
        }
    }
  }

  const errorInfo = getErrorInfo()
  const Icon = errorInfo.icon

  const handleRetry = () => {
    window.location.href = `/${appName}/`
  }

  return (
    <div className="flex flex-col items-center justify-center h-full min-h-[60vh] p-4">
      <div className="text-center max-w-lg">
        <div className="flex justify-center mb-6">
          <div className="p-4 bg-red-100 dark:bg-red-900/30 rounded-full">
            <Icon size={48} className="text-red-500 dark:text-red-400" strokeWidth={1.5} />
          </div>
        </div>

        <h1 className="text-3xl font-bold text-gray-900 dark:text-white">
          {errorInfo.title}
        </h1>

        <p className="mt-4 text-gray-600 dark:text-gray-400">
          {errorInfo.description}
        </p>

        {details && (
          <div className="mt-4 p-3 bg-gray-100 dark:bg-gray-800 rounded-lg text-left">
            <p className="text-xs font-mono text-gray-500 dark:text-gray-500 break-all">
              {decodeURIComponent(details)}
            </p>
          </div>
        )}

        <div className="mt-8 flex items-center justify-center gap-3 flex-wrap">
          <button
            onClick={handleRetry}
            className="flex items-center gap-2 px-4 py-2 text-white bg-blue-600 hover:bg-blue-700 rounded-lg transition-colors"
          >
            <RefreshCw size={18} />
            Try Again
          </button>

          <button
            onClick={() => window.history.back()}
            className="flex items-center gap-2 px-4 py-2 text-gray-700 dark:text-gray-300 bg-gray-200 dark:bg-gray-700 hover:bg-gray-300 dark:hover:bg-gray-600 rounded-lg transition-colors"
          >
            <ArrowLeft size={18} />
            Go Back
          </button>

          <Link
            to="/"
            className="flex items-center gap-2 px-4 py-2 text-gray-700 dark:text-gray-300 bg-gray-200 dark:bg-gray-700 hover:bg-gray-300 dark:hover:bg-gray-600 rounded-lg transition-colors"
          >
            <Home size={18} />
            Dashboard
          </Link>
        </div>

        <p className="mt-6 text-sm text-gray-500 dark:text-gray-500">
          If this issue persists, check the app status in the{' '}
          <Link to="/apps" className="text-blue-600 hover:underline">
            Apps page
          </Link>
          .
        </p>
      </div>
    </div>
  )
}
