/// <reference types="vitest/config" />
import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'
import { readFileSync } from 'fs'
import { resolve } from 'path'
import { createConnection } from 'net'

const APP_VERSION = (
  JSON.parse(readFileSync(resolve(__dirname, 'package.json'), 'utf8')) as { version: string }
).version

// Where the dev server forwards backend traffic. The built bundle is served by
// the gateway itself (same origin), so this proxy only matters under `npm run
// dev`, where Vite serves the UI on its own port.
//
// The gateway's conventional port is 18790, but a second instance (or an agent
// session avoiding a clash) binds elsewhere — e.g. `gateway --port 18801`. With
// a hard-coded target the dev UI then silently shows "DISCONNECTED" against a
// perfectly healthy gateway, which reads as a product bug. So: honour an
// explicit override, prefer the conventional port, and otherwise discover the
// live loopback gateway before falling back to the default.
// 端口单一事实源：core/src/gateway/mod.rs 的 `DEFAULT_PORT`（编译期常量不变量）。
// 改端口须同步该真源 + gui/src-tauri/tauri.conf.json（由 gui/src-tauri/build.rs 构建期强制校验）。
const CONVENTIONAL_GATEWAY_PORT = 18790
const DISCOVERY_RANGE = 24 // 18790..18813 — covers ad-hoc `--port 188xx` instances

function portAccepts(port: number, timeoutMs = 180): Promise<boolean> {
  return new Promise((done) => {
    const sock = createConnection({ host: '127.0.0.1', port })
    const finish = (ok: boolean) => {
      sock.removeAllListeners()
      sock.destroy()
      done(ok)
    }
    sock.setTimeout(timeoutMs)
    sock.once('connect', () => finish(true))
    sock.once('timeout', () => finish(false))
    sock.once('error', () => finish(false))
  })
}

/** Resolve the gateway origin to proxy to. Only probes while serving (`npm run dev`). */
async function resolveGatewayTarget(command: string): Promise<string> {
  const explicit = process.env.OPENSQUILLA_GATEWAY_URL
  if (explicit) return explicit
  if (command !== 'serve') return `http://127.0.0.1:${CONVENTIONAL_GATEWAY_PORT}`

  const candidates = Array.from({ length: DISCOVERY_RANGE }, (_, i) => CONVENTIONAL_GATEWAY_PORT + i)
  const reachable = await Promise.all(candidates.map((p) => portAccepts(p)))
  const found = candidates.find((_, i) => reachable[i])
  if (found && found !== CONVENTIONAL_GATEWAY_PORT) {
    console.log(`[vite] gateway discovery: 127.0.0.1:${CONVENTIONAL_GATEWAY_PORT} is idle → proxying to 127.0.0.1:${found}`)
  }
  return `http://127.0.0.1:${found ?? CONVENTIONAL_GATEWAY_PORT}`
}

// https://vitejs.dev/config/
export default defineConfig(async ({ command }) => {
  const gatewayTarget = await resolveGatewayTarget(command)
  return {
    plugins: [vue()],
    base: './',
    // vue-i18n build feature flags — silences the bundler warnings and drops the
    // legacy API + prod devtools from the bundle (Composition mode only).
    define: {
      __VUE_I18N_FULL_INSTALL__: 'true',
      __VUE_I18N_LEGACY_API__: 'false',
      __INTLIFY_PROD_DEVTOOLS__: 'false',
      __APP_VERSION__: JSON.stringify(APP_VERSION),
    },
    server: {
      proxy: {
        // REST endpoints (approvals, artifacts under /api/v1, file upload, audio, …).
        // Strip the browser's Origin header on the way through: the gateway's
        // same-origin guard on state-changing routes would otherwise reject
        // proxied POSTs (Origin says localhost:5173, Host says the gateway).
        // Requests through this proxy are developer-initiated, not drive-by.
        '/api': {
          target: gatewayTarget,
          changeOrigin: true,
          configure: (proxy) => {
            proxy.on('proxyReq', (proxyReq) => proxyReq.removeHeader('origin'))
          },
        },
        // RpcClient connects to ws://<host>/ws for the live chat/event stream.
        '/ws': { target: gatewayTarget, ws: true, changeOrigin: true },
        // Backend-owned static assets (brand mark, share-export images, …) that the
        // app loads from `${base}/static/*`. Scope to /control/static ONLY — the
        // bare /control prefix is the SPA router base and must stay with Vite, or
        // proxying it would forward the app's routes to the gateway and defeat HMR.
        '/control/static': { target: gatewayTarget, changeOrigin: true },
      },
    },
    build: {
      outDir: resolve(__dirname, 'dist'),
      emptyOutDir: true,
      // Never on: `sourcesContent` puts every hand-written .vue/.ts file back
      // together on the user's disk, and this build ships inside the installer.
      // Measured on the published 0.2.17 MSI: 206/206 maps carried sources.
      sourcemap: false,
      rollupOptions: {
        output: {
          assetFileNames: (assetInfo) => {
            const info = assetInfo.name?.split('.') || []
            const ext = info[info.length - 1]
            if (/\.(png|jpe?g|gif|svg|webp|ico)$/i.test(assetInfo.name || '')) {
              return `assets/img/[name]-[hash][extname]`
            }
            if (/\.(woff2?|ttf|otf|eot)$/i.test(assetInfo.name || '')) {
              return `assets/fonts/[name]-[hash][extname]`
            }
            return `assets/[name]-[hash][extname]`
          },
          chunkFileNames: 'assets/[name]-[hash].js',
          entryFileNames: 'assets/[name]-[hash].js',
        },
      },
    },
    resolve: {
      alias: {
        '@': resolve(__dirname, 'src'),
      },
    },
    // Vitest: pure unit/property tests for the chat reducer and the merge/parity
    // helpers. Node environment (no DOM needed); the Playwright e2e tree under
    // e2e/ has its own runner and is excluded.
    test: {
      environment: 'node',
      include: ['src/**/*.{test,spec}.ts'],
      exclude: ['e2e/**', 'node_modules/**'],
      // A handful of component suites (SettingsDialog and friends) re-import the
      // whole dialog tree under `vi.resetModules()`, which costs ~1.5s per mount
      // in isolation. With the full file set running in parallel that comfortably
      // exceeds Vitest's 5s default on a loaded machine, and a timeout there is
      // never a real defect — just a slow worker. Raise the ceiling so CI reports
      // genuine failures instead of scheduling noise. Tests that hang for real
      // still fail, just 15s later.
      testTimeout: 15000,
    },
  }
})
