import init, { Document, prewarm, slugs, version } from 'aozora-wasm';

let initialized = false;
let initPromise: Promise<void> | null = null;

export function ensureWasmReady(): Promise<void> {
  if (initialized) return Promise.resolve();
  if (initPromise) return initPromise;
  const p = init()
    .then(() => {
      prewarm();
      initialized = true;
    })
    .catch((error: unknown) => {
      initPromise = null;
      throw error;
    });
  initPromise = p;
  return p;
}

export { Document, prewarm, slugs, version };
