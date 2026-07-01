import { defineConfig, devices } from '@playwright/test';

// E2E harness for the aozora playground (#335 D-5). Runs against a production
// `vite preview` build so the real WASM parse engine is exercised end-to-end
// (the vitest unit suite stubs it). The build serves under the prod base path
// `/aozora/playground/` (see vite.config.ts `base`); `vite preview` defaults to
// port 4173.
const BASE = 'http://localhost:4173/aozora/playground/';

export default defineConfig({
  testDir: './e2e',
  fullyParallel: true,
  // A stray `test.only` must fail CI, never silently narrow the run.
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 2 : 0,
  // One worker locally keeps the single preview server calm; CI parallelises.
  workers: process.env.CI ? undefined : 1,
  reporter: process.env.CI ? [['github'], ['line']] : 'line',
  use: {
    baseURL: BASE,
    trace: 'on-first-retry',
  },
  projects: [{ name: 'chromium', use: { ...devices['Desktop Chrome'] } }],
  // `vite preview` serves the built `dist/` — it does NOT build first, so the
  // command builds then previews. `bun run build` needs the WASM `pkg/` present
  // (the `just playground-e2e` recipe / CI `e2e` job build it via wasm-pack
  // beforehand).
  webServer: {
    command: 'bun run build && bun run preview',
    url: BASE,
    reuseExistingServer: !process.env.CI,
    timeout: 180_000,
  },
});
