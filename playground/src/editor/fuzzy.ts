const INITIAL_PREVIOUS_MATCH_INDEX = -2;
const CONTIGUOUS_MATCH_BONUS = 3;
const WORD_BOUNDARY_BONUS = 2;
const EARLY_MATCH_WINDOW = 8;
const EARLY_MATCH_WEIGHT = 0.1;

export interface FuzzyMatch {
  /** Higher is better. */
  score: number;
  /** Matching code point indices in the lowercase target. */
  indices: number[];
}

/**
 * Score `query` as a case-insensitive subsequence of `target`.
 * Return `null` for a nonmatch and a zero score for an empty query.
 * Contiguous matches and word boundaries increase the score.
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
  let prev = INITIAL_PREVIOUS_MATCH_INDEX;
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
    if (found === prev + 1) score += CONTIGUOUS_MATCH_BONUS;
    const before = found > 0 ? tChars[found - 1]! : '';
    if (found === 0 || /[\s\-_/.]/.test(before)) score += WORD_BOUNDARY_BONUS;
    score += Math.max(0, EARLY_MATCH_WINDOW - found) * EARLY_MATCH_WEIGHT;
    prev = found;
    ti = found + 1;
  }
  return { score, indices };
}

/**
 * Rank matching items by descending score on `key(item)`.
 * Preserve input order for ties and blank queries.
 */
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
