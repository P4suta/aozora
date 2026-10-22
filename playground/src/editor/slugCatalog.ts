import type { Slug as SlugEntry } from 'aozora-wasm';
import { slugs } from '../wasm-loader';

let cache: SlugEntry[] | null = null;

/** Requires `ensureWasmReady()` to complete before the first call. */
export function loadSlugCatalog(): SlugEntry[] {
  if (cache) return cache;
  cache = Array.from(slugs());
  return cache;
}

export type { SlugEntry };
