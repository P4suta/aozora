import init, { Document, slugs_json } from 'aozora-wasm';

let initialized = false;
let initPromise: Promise<void> | null = null;

export function ensureWasmReady(): Promise<void> {
  if (initialized) return Promise.resolve();
  if (initPromise) return initPromise;
  const p = init().then(() => {
    initialized = true;
  });
  initPromise = p;
  return p;
}

export { Document, slugs_json };
