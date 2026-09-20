import { defineConfig } from 'vite';
import appConfig from './vite.config';

export default defineConfig({
  ...appConfig,
  // Even a missing browser mock must never reach the development API proxy.
  server: { host: '127.0.0.1', port: 4174, strictPort: true, proxy: {} },
});
