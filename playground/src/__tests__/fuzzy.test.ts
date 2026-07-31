import { describe, expect, it } from 'vitest';
import { fuzzyMatch, fuzzyRank } from '../editor/fuzzy';

describe('fuzzy', () => {
  describe('fuzzyMatch', () => {
    it('returns a score and positions for a subsequence match', () => {
      const m = fuzzyMatch('rb', 'ruby');
      expect(m).not.toBeNull();
      expect(m!.indices).toEqual([0, 2]);
    });

    it('returns null when the query is not a subsequence', () => {
      expect(fuzzyMatch('xyz', 'ruby')).toBeNull();
    });

    it('ignores letter case', () => {
      expect(fuzzyMatch('RUBY', 'ruby')).not.toBeNull();
    });

    it('matches an empty query with a zero score', () => {
      expect(fuzzyMatch('', 'anything')).toEqual({ score: 0, indices: [] });
    });

    it('scores contiguous matches above scattered word-start matches', () => {
      const contiguous = fuzzyMatch('ru', 'ruby')!;
      const scattered = fuzzyMatch('ru', 'rocky_underground')!;
      expect(contiguous.score).toBeGreaterThan(scattered.score);
    });

    it('matches Japanese subsequences', () => {
      expect(fuzzyMatch('ルビ', 'ルビ')).not.toBeNull();
      expect(fuzzyMatch('点', '傍点')).not.toBeNull();
    });
  });

  describe('fuzzyRank', () => {
    const items = [
      { id: 'aozora.wrap.ruby', description: 'ルビ' },
      { id: 'aozora.wrap.bouten', description: '傍点' },
      { id: 'aozora.wrap.chuki', description: '注記で囲む' },
    ];
    const key = (i: { id: string; description: string }) =>
      `${i.id} ${i.description}`;

    it('ranks an ID match first for a romaji query', () => {
      const r = fuzzyRank('ruby', items, key);
      expect(r[0]!.id).toBe('aozora.wrap.ruby');
    });

    it('matches descriptions for Japanese queries', () => {
      const r = fuzzyRank('傍点', items, key);
      expect(r[0]!.id).toBe('aozora.wrap.bouten');
    });

    it('preserves item order for an empty query', () => {
      expect(fuzzyRank('', items, key).map((i) => i.id)).toEqual(
        items.map((i) => i.id),
      );
    });

    it('returns an empty array when nothing matches', () => {
      expect(fuzzyRank('zzzz', items, key)).toHaveLength(0);
    });
  });
});
