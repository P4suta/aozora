import { describe, expect, it } from 'vitest';
import { lowerBoundByStart } from '../editor/utils';

describe('lowerBoundByStart', () => {
  const entries = [
    { span: { start: 0 } },
    { span: { start: 5 } },
    { span: { start: 10 } },
    { span: { start: 20 } },
  ];

  it('returns zero for an empty array', () => {
    expect(lowerBoundByStart([], 5)).toBe(0);
  });

  it('returns zero before every entry', () => {
    expect(lowerBoundByStart(entries, -1)).toBe(0);
  });

  it('returns the matching start index for an exact match', () => {
    expect(lowerBoundByStart(entries, 5)).toBe(1);
    expect(lowerBoundByStart(entries, 10)).toBe(2);
  });

  it('returns the index after the final entry below the byte offset', () => {
    expect(lowerBoundByStart(entries, 7)).toBe(2); // 5 < 7 < 10
    expect(lowerBoundByStart(entries, 15)).toBe(3); // 10 < 15 < 20
  });

  it('returns the array length after every entry', () => {
    expect(lowerBoundByStart(entries, 100)).toBe(4);
  });
});
