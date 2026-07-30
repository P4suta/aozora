interface HasSpanStart {
  span: { start: number };
}

export function lowerBoundByStart<T extends HasSpanStart>(
  entries: T[],
  byte: number,
): number {
  let lo = 0;
  let hi = entries.length;
  while (lo < hi) {
    const mid = (lo + hi) >>> 1;
    if (entries[mid]!.span.start < byte) lo = mid + 1;
    else hi = mid;
  }
  return lo;
}
