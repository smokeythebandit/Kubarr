import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'
import { execSync } from 'child_process'
import { readFileSync } from 'fs'
import { join } from 'path'

// Get version from environment or package.json
const getVersion = () => {
  // Check environment variable first (set in Docker build)
  if (process.env.VITE_VERSION && process.env.VITE_VERSION !== '0.0.0') {
    return process.env.VITE_VERSION
  }
  // Fall back to package.json for local development
  try {
    const pkg = JSON.parse(readFileSync(join(__dirname, 'package.json'), 'utf-8'))
    return pkg.version
  } catch {
    return '0.0.0'
  }
}

// Get git commit hash at build time
const getGitHash = () => {
  // Check environment variable first (set in Docker build)
  if (process.env.VITE_COMMIT_HASH && process.env.VITE_COMMIT_HASH !== 'unknown') {
    return process.env.VITE_COMMIT_HASH
  }
  // Fall back to git command for local development
  try {
    return execSync('git rev-parse --short HEAD').toString().trim()
  } catch {
    return 'unknown'
  }
}

// Get build time
const getBuildTime = () => {
  if (process.env.VITE_BUILD_TIME && process.env.VITE_BUILD_TIME !== 'unknown') {
    return process.env.VITE_BUILD_TIME
  }
  return new Date().toISOString()
}

// Get release channel (dev, release, stable)
const getChannel = () => {
  return process.env.VITE_CHANNEL || 'dev'
}

// https://vitejs.dev/config/
export default defineConfig({
  plugins: [tailwindcss(), react()],
  base: '/',
  define: {
    __VERSION__: JSON.stringify(getVersion()),
    __COMMIT_HASH__: JSON.stringify(getGitHash()),
    __BUILD_TIME__: JSON.stringify(getBuildTime()),
    __CHANNEL__: JSON.stringify(getChannel()),
  },
  server: {
    port: 5173,
    proxy: {
      '/api': {
        target: 'http://localhost:8080',
        changeOrigin: true,
        ws: true,
      },
      '/auth': {
        target: 'http://localhost:8080',
        changeOrigin: true,
      },
    },
  },
})
