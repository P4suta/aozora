import type { Slug as SlugEntry } from 'aozora-wasm';
import { slugs } from '../wasm-loader';

let cache: SlugEntry[] | null = null;

/**
 * Load the slug catalogue from the WASM module. Idempotent: the
 * first call serialises via `aozora-wasm` and parses the envelope;
 * subsequent calls return the cached array.
 *
 * Must be called after `ensureWasmReady()`.
 */
export function loadSlugCatalog(): SlugEntry[] {
  if (cache) return cache;
  cache = Array.from(slugs());
  return cache;
}

export type { SlugEntry };
