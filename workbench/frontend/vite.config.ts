import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

// Dev server proxies /api -> backend on :8011 so the SPA can also run without
// cross-origin configuration. The API client also supports a direct base URL
// via VITE_API_BASE for production builds.
export default defineConfig({
  plugins: [react()],
  server: {
    port: 5173,
    proxy: {
      '/api': {
        target: 'http://localhost:8011',
        changeOrigin: true,
      },
      // Introspection endpoints live outside the /api prefix on the backend;
      // the SPA reaches them via `../health` / `../harness/capabilities`
      // relative to its API base, which normalizes to these paths in dev.
      '/health': {
        target: 'http://localhost:8011',
        changeOrigin: true,
      },
      '/harness/capabilities': {
        target: 'http://localhost:8011',
        changeOrigin: true,
      },
    },
  },
  build: {
    target: 'es2020',
    sourcemap: false,
  },
});
