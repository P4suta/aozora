import { defineConfig } from 'vitest/config';
import { fileURLToPath } from 'node:url';

/**
 * Vitest 設定。Vite の resolve.alias は vite.config.ts で `aozora-wasm`
 * を実体に向けているが、test では WASM をロードする純粋関数のみを対象に
 * するため alias は不要。
 *
 * environment は happy-dom — localStorage / TextEncoder / DOM API の
 * 軽量な実装を提供し、jsdom より起動が速い。
 */
export default defineConfig({
  resolve: {
    alias: {
      // Test files do not import aozora-wasm directly; this is a
      // safety net so a stray import inside a tested module compiles
      // (it will not execute on the test path).
      'aozora-wasm': fileURLToPath(
        new URL('./src/__tests__/__stubs__/aozora-wasm.ts', import.meta.url),
      ),
    },
  },
  test: {
    // localStorage を使うテストは自前 polyfill を import する形にした
    // （vitest 4 で `environment: 'happy-dom'` が global を inject しないケースが
    // あったため）。pure 関数のテストは node 環境で十分。
    environment: 'node',
    globals: false,
    include: ['src/**/*.test.ts'],
    reporters: 'default',
    // Process CSS so `?raw` imports resolve to real file text. Vitest stubs
    // CSS to an empty module by default (even for `?raw`), which would make
    // the theme-token audit read an empty styles.css. No test imports CSS for
    // its styles, so enabling this only affects the `?raw` text reads.
    css: true,
  },
});
