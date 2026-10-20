import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'

// Dev: `npm run dev` (port 5173) proxies the API to the engine on :8000 (`cargo run -p yt2midi-core` in app/).
// Prod: `npm run build`, then the program embeds frontend/dist itself.
export default defineConfig({
  plugins: [react()],
  server: { port: 5173, proxy: { '/api': 'http://127.0.0.1:8000' } },
  build: { target: 'es2022', sourcemap: false },
})
