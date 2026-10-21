import { readFileSync } from 'node:fs'
import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'

const packageJson = JSON.parse(readFileSync(new URL('./package.json', import.meta.url), 'utf8'))

// Development-only CSP relaxation: Vite injects an inline react-refresh preamble and
// uses a WebSocket for HMR, so the production CSP written in index.html would block
// the dev server. The production build keeps the strict policy.
function devCspPlugin() {
  return {
    name: 'vpn233-dev-csp',
    apply: 'serve',
    transformIndexHtml(html) {
      return html.replace(
        /<meta\s+http-equiv="Content-Security-Policy"\s+content="[^"]*"\s*\/?>/,
        `<meta http-equiv="Content-Security-Policy" content="default-src 'self'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'self' http://127.0.0.1:* http://localhost:* ws://127.0.0.1:* ws://localhost:*; frame-ancestors 'none'" />`,
      )
    },
  }
}

export default defineConfig({
  plugins: [react(), devCspPlugin()],
  define: {
    __APP_VERSION__: JSON.stringify(packageJson.version),
  },
  // The console ships as a single authenticated bundle; the raw budget is enforced
  // by scripts/check-dist.mjs (900 KB), so keep the build log free of size warnings.
  build: { chunkSizeWarningLimit: 900 },
  server: { port: 4173 },
})
