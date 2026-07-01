import { describe, expect, it } from 'vitest';
import { fuzzyMatch, fuzzyRank } from '../editor/fuzzy';

describe('fuzzy', () => {
  describe('fuzzyMatch', () => {
    it('部分列に一致すればスコアと一致位置を返す', () => {
      const m = fuzzyMatch('rb', 'ruby');
      expect(m).not.toBeNull();
      expect(m!.indices).toEqual([0, 2]);
    });

    it('部分列でなければ null', () => {
      expect(fuzzyMatch('xyz', 'ruby')).toBeNull();
    });

    it('大文字小文字を無視する', () => {
      expect(fuzzyMatch('RUBY', 'ruby')).not.toBeNull();
    });

    it('空クエリは全一致（スコア0）', () => {
      expect(fuzzyMatch('', 'anything')).toEqual({ score: 0, indices: [] });
    });

    it('連続一致は語頭飛び一致よりスコアが高い', () => {
      const contiguous = fuzzyMatch('ru', 'ruby')!;
      const scattered = fuzzyMatch('ru', 'rocky_underground')!;
      expect(contiguous.score).toBeGreaterThan(scattered.score);
    });

    it('日本語の部分列も一致する', () => {
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
    const key = (i: { id: string; description: string }) => `${i.id} ${i.description}`;

    it('romaji クエリで id に一致する項目を先頭に返す', () => {
      const r = fuzzyRank('ruby', items, key);
      expect(r[0]!.id).toBe('aozora.wrap.ruby');
    });

    it('日本語クエリで description に一致する', () => {
      const r = fuzzyRank('傍点', items, key);
      expect(r[0]!.id).toBe('aozora.wrap.bouten');
    });

    it('空クエリは全項目を順序保持で返す', () => {
      expect(fuzzyRank('', items, key).map((i) => i.id)).toEqual(items.map((i) => i.id));
    });

    it('一致しないクエリは空配列', () => {
      expect(fuzzyRank('zzzz', items, key)).toHaveLength(0);
    });
  });
});
