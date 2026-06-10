import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'

// https://vite.dev/config/
export default defineConfig({
  // Relative base so the built SPA can be served from any path (e.g. embedded
  // and served by the Rust control plane, or under a sub-path).
  base: './',
  plugins: [react(), tailwindcss()],
  server: {
    port: 5173,
    // The portal talks to the control plane only through this prefix. In dev it
    // proxies to the local axum API (override the port with BIFROST_API_TARGET);
    // the mock client is the default until the backend is running.
    proxy: {
      '/api': {
        target: process.env.BIFROST_API_TARGET ?? 'http://127.0.0.1:8080',
        changeOrigin: true,
      },
    },
  },
})
