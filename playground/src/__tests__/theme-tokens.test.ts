import { describe, it, expect } from 'vitest';

/**
 * Theme-token audit — a mechanical guard for the CSS custom-property system
 * that stylelint cannot express.
 *
 * Two invariants:
 *   (a) light/dark parity — every colour token defined in the light `:root`
 *       block is also defined in the `:root[data-theme='dark']` block (and
 *       vice-versa), so a new token can't ship half-themed. Theme-independent
 *       tokens (fonts, z-index scale) are defined only once, in light, and
 *       are excluded via an allowlist.
 *   (b) no dead tokens — every `--x` defined in styles.css is referenced by a
 *       `var(--x)` somewhere in the app.
 *
 * Files are read through Vite's `?raw` loader (typed by `vite/client`, which
 * is already in tsconfig `types`) rather than `node:fs` — the app tsconfig has
 * no `@types/node`, so `fs` would fail typecheck, and `?raw` also removes all
 * cwd/path fragility.
 *
 * DEFINED tokens are scanned from styles.css ONLY: aozora.css defines a
 * separate `--aozora-*` bridge namespace consumed by the canonical sheet
 * (outside this scan), so including it would false-flag those as dead.
 *
 * REFERENCED tokens are scanned across every src file EXCEPT the tests — and
 * `.ts`/`.tsx` are non-negotiable: `src/editor/theme.ts` (the CodeMirror
 * theme) is the sole referencer of `--success`, `--accent-selection`,
 * `--token-ruby`, `--token-bouten` and `--token-bouten-bg`. A CSS-only scan
 * would report all five as dead.
 */

// Raw source of every stylesheet + TS/TSX module under src/.
const sources = import.meta.glob('../**/*.{css,ts,tsx}', {
  query: '?raw',
  eager: true,
  import: 'default',
}) as Record<string, string>;

// Tokens that are intentionally theme-independent: defined once (in light
// `:root`) and reused as-is in dark. Excluded from the parity check.
const THEME_INDEPENDENT = new Set([
  '--font-ui',
  '--font-mono',
  '--font-serif',
  '--z-dropdown',
  '--z-modal',
  '--z-toast',
]);

const stylesKey = Object.keys(sources).find((k) => k.endsWith('/styles.css'));
if (!stylesKey) throw new Error('theme-tokens: styles.css not found via import.meta.glob');
const styles = sources[stylesKey];

/** Body of the first CSS block whose selector matches `re` (flat blocks only). */
function blockBody(re: RegExp): string {
  const m = styles.match(re);
  if (!m) throw new Error(`theme-tokens: block not found for ${re}`);
  return m[1];
}

/** Custom-property *definitions* (`--x:`) inside a block. */
function definitions(body: string): Set<string> {
  const set = new Set<string>();
  for (const m of body.matchAll(/(--[\w-]+)\s*:/g)) set.add(m[1]);
  return set;
}

/** Custom-property *references* (`var(--x)`) anywhere in `src`. */
function references(src: string): Set<string> {
  const set = new Set<string>();
  for (const m of src.matchAll(/var\(\s*(--[\w-]+)/g)) set.add(m[1]);
  return set;
}

const lightTokens = definitions(blockBody(/:root\s*\{([^}]*)\}/));
const darkTokens = definitions(blockBody(/:root\[data-theme='dark'\]\s*\{([^}]*)\}/));

const referenced = new Set<string>();
for (const [key, src] of Object.entries(sources)) {
  if (key.includes('__tests__')) continue; // don't let the test's own token literals mask dead ones
  for (const t of references(src)) referenced.add(t);
}

describe('theme tokens', () => {
  it('extracts a plausible token set (guards the parser against CSS reformatting)', () => {
    expect(lightTokens.size).toBeGreaterThan(10);
    expect(darkTokens.size).toBeGreaterThan(10);
    // The allowlist must name real light-`:root` tokens, or it is stale.
    for (const t of THEME_INDEPENDENT) {
      expect(lightTokens.has(t), `${t} is allowlisted but not defined in :root`).toBe(true);
    }
  });

  it('light and dark define the same themed tokens (parity)', () => {
    const themed = (s: Set<string>) => [...s].filter((t) => !THEME_INDEPENDENT.has(t)).sort();
    const missingFromDark = themed(lightTokens).filter((t) => !darkTokens.has(t));
    const missingFromLight = themed(darkTokens).filter((t) => !lightTokens.has(t));
    expect(missingFromDark, 'themed tokens defined in light but missing from dark').toEqual([]);
    expect(missingFromLight, 'themed tokens defined in dark but missing from light').toEqual([]);
  });

  it('has no dead tokens (every defined --x is referenced via var(--x))', () => {
    const defined = new Set([...lightTokens, ...darkTokens]);
    const dead = [...defined].filter((t) => !referenced.has(t)).sort();
    expect(dead, 'defined tokens with no var(--x) consumer').toEqual([]);
  });
});
