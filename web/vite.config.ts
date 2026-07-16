import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'

export default defineConfig({
  plugins: [react()],
  server: {
    proxy: {
      '/api': {
        target: 'http://127.0.0.1:8321',
        changeOrigin: true,
        // Strip the browser Origin so the backend's same-origin guard sees a
        // clean same-origin request (Host is rewritten to the target by
        // changeOrigin). Alternatively run the server with
        // --allowed-origin http://localhost:5173
        configure: (proxy) => {
          proxy.on('proxyReq', (proxyReq) => proxyReq.removeHeader('origin'))
        },
      },
    },
  },
})
