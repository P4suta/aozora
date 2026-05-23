import { slugs_json } from '../wasm-loader';
import { warn } from '../logger';

export interface SlugEntry {
  canonical: string;
  family: string;
  accepts_param: boolean;
  doc: string;
  partner: string | null;
}

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
  try {
    const env = JSON.parse(slugs_json()) as {
      schema_version: number;
      data: SlugEntry[];
    };
    cache = env.data ?? [];
  } catch (err) {
    warn('Failed to load slug catalog from WASM:', err);
    cache = [];
  }
  return cache;
}
