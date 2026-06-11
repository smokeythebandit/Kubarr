import { Link } from 'react-router-dom'
import { Home, ArrowLeft, Ship } from 'lucide-react'

export default function NotFoundPage() {
  return (
    <div className="flex flex-col items-center justify-center h-full min-h-[60vh] p-4">
      <div className="text-center">
        <div className="flex justify-center mb-6">
          <Ship size={64} className="text-blue-500 dark:text-blue-400" strokeWidth={1.5} />
        </div>

        <h1 className="text-8xl font-bold text-gray-200 dark:text-gray-700">404</h1>

        <h2 className="mt-4 text-2xl font-semibold text-gray-900 dark:text-white">
          Page Not Found
        </h2>

        <p className="mt-2 text-gray-600 dark:text-gray-400 max-w-md mx-auto">
          The page you're looking for doesn't exist or has been moved.
        </p>

        <div className="mt-8 flex items-center justify-center gap-4">
          <button
            onClick={() => window.history.back()}
            className="flex items-center gap-2 px-4 py-2 text-gray-700 dark:text-gray-300 bg-gray-200 dark:bg-gray-700 hover:bg-gray-300 dark:hover:bg-gray-600 rounded-lg transition-colors"
          >
            <ArrowLeft size={18} />
            Go Back
          </button>

          <Link
            to="/"
            className="flex items-center gap-2 px-4 py-2 text-white bg-blue-600 hover:bg-blue-700 rounded-lg transition-colors"
          >
            <Home size={18} />
            Home
          </Link>
        </div>
      </div>
    </div>
  )
}
