// Pure subsequence fuzzy matcher for the command palette (#334 D-4). No deps —
// CM6/Solid-agnostic so it unit-tests without a DOM.

export interface FuzzyMatch {
  /** Higher is better. */
  score: number;
  /** Byte-agnostic char indices in the target that matched, for highlighting. */
  indices: number[];
}

/**
 * Score `query` against `target` as a case-insensitive subsequence. Returns
 * `null` when `query` is not a subsequence of `target`. Contiguous runs and
 * word-boundary starts rank higher; an empty query matches everything at 0.
 */
export function fuzzyMatch(query: string, target: string): FuzzyMatch | null {
  if (query === '') return { score: 0, indices: [] };
  const q = query.toLowerCase();
  const t = target.toLowerCase();
  const tChars = [...t];
  const qChars = [...q];
  const indices: number[] = [];
  let ti = 0;
  let score = 0;
  let prev = -2;
  for (const ch of qChars) {
    let found = -1;
    for (; ti < tChars.length; ti++) {
      if (tChars[ti] === ch) {
        found = ti;
        break;
      }
    }
    if (found === -1) return null;
    indices.push(found);
    if (found === prev + 1) score += 3; // contiguous
    const before = found > 0 ? tChars[found - 1]! : '';
    if (found === 0 || /[\s\-_/.]/.test(before)) score += 2; // word boundary
    score += Math.max(0, 8 - found) * 0.1; // earlier is marginally better
    prev = found;
    ti = found + 1;
  }
  return { score, indices };
}

/** Rank `items` by fuzzy match on `key(item)`, best first; drops non-matches. */
export function fuzzyRank<T>(
  query: string,
  items: readonly T[],
  key: (item: T) => string,
): T[] {
  if (query.trim() === '') return [...items];
  const scored: Array<{ item: T; score: number }> = [];
  for (const item of items) {
    const m = fuzzyMatch(query, key(item));
    if (m) scored.push({ item, score: m.score });
  }
  scored.sort((a, b) => b.score - a.score);
  return scored.map((s) => s.item);
}
