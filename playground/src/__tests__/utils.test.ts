import { describe, expect, it } from 'vitest';
import { lowerBoundByStart } from '../editor/utils';

describe('lowerBoundByStart', () => {
  const entries = [
    { span: { start: 0 } },
    { span: { start: 5 } },
    { span: { start: 10 } },
    { span: { start: 20 } },
  ];

  it('空配列では 0 を返す', () => {
    expect(lowerBoundByStart([], 5)).toBe(0);
  });

  it('全要素より小さい値では 0', () => {
    expect(lowerBoundByStart(entries, -1)).toBe(0);
  });

  it('完全一致では同じ start のインデックス', () => {
    expect(lowerBoundByStart(entries, 5)).toBe(1);
    expect(lowerBoundByStart(entries, 10)).toBe(2);
  });

  it('間の値では「< byte」を満たす最後の次', () => {
    expect(lowerBoundByStart(entries, 7)).toBe(2); // 5 < 7 < 10
    expect(lowerBoundByStart(entries, 15)).toBe(3); // 10 < 15 < 20
  });

  it('全要素より大きい値では length', () => {
    expect(lowerBoundByStart(entries, 100)).toBe(4);
  });
});
