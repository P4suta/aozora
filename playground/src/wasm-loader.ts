import init, { Document, slugsJson, prewarm, version } from 'aozora-wasm';

let initialized = false;
let initPromise: Promise<void> | null = null;

export function ensureWasmReady(): Promise<void> {
  if (initialized) return Promise.resolve();
  if (initPromise) return initPromise;
  const p = init().then(() => {
    // Warm the parser tables (SIMD backend choice + annotation-classifier
    // DFA) right after init() resolves — before the editor is created, and
    // thus before any keystroke triggers a parse — so the first parse
    // doesn't pay the one-time build cost on the main thread.
    prewarm();
    initialized = true;
  });
  initPromise = p;
  return p;
}

export { Document, slugsJson, prewarm, version };
