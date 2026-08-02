export interface PositionLike {
  readonly line: number;
  readonly character: number;
}

export interface RangeLike {
  readonly start: PositionLike;
  readonly end: PositionLike;
}

export interface OffsetMatch {
  readonly start: number;
  readonly end: number;
  readonly body: string;
}

export interface TextReplacement {
  readonly start: number;
  readonly end: number;
  readonly text: string;
  readonly cursorOffset: number;
}

export function canonicalizeSnapshotMatches(
  expectedVersion: number,
  currentVersion: number,
  expectedBody: string,
  currentBody: string,
): boolean {
  return expectedVersion === currentVersion && expectedBody === currentBody;
}

export function documentVersionMatches(expectedVersion: number, currentVersion: number): boolean {
  return expectedVersion === currentVersion;
}

export function htmlFileName(sourceName: string): string {
  const leaf = sourceName.split(/[/\\]/u).pop() || "aozora";
  const stem = leaf.replace(/\.(?:afm|aozora(?:\.txt)?|txt|text)$/iu, "");
  return `${stem || "aozora"}.html`;
}

const SLUG_PATTERN = /[［[][＃#][^］\]\n]*[］\]]/g;

export function findSlugAtOffset(line: string, offset: number): OffsetMatch | undefined {
  for (const match of line.matchAll(SLUG_PATTERN)) {
    const start = match.index ?? 0;
    const end = start + match[0].length;
    if (offset >= start && offset < end) {
      return { start, end, body: match[0] };
    }
  }
  return undefined;
}

export function compareRangesDescending(a: RangeLike, b: RangeLike): number {
  return (
    b.start.line - a.start.line ||
    b.start.character - a.start.character ||
    b.end.line - a.end.line ||
    b.end.character - a.end.character
  );
}

function comparePositions(a: PositionLike, b: PositionLike): number {
  return a.line - b.line || a.character - b.character;
}

export function anyPositionInRange(
  positions: ReadonlyArray<PositionLike>,
  range: RangeLike,
): boolean {
  return positions.some(
    (position) =>
      comparePositions(position, range.start) >= 0 && comparePositions(position, range.end) < 0,
  );
}

export function expandWrapText(
  template: string,
  selected: string,
): {
  readonly text: string;
  readonly cursorOffset: number;
} {
  const marker = template.indexOf("$0");
  if (marker < 0 || template.indexOf("$0", marker + 2) >= 0) {
    throw new Error("wrap template must contain one $0 marker");
  }
  const before = template.slice(0, marker).split("BASE").join(selected);
  const after = template
    .slice(marker + 2)
    .split("BASE")
    .join(selected);
  return { text: before + after, cursorOffset: before.length };
}

export function finalCursorOffsets(replacements: ReadonlyArray<TextReplacement>): number[] {
  const ordered = replacements
    .map((replacement, index) => ({ replacement, index }))
    .sort(
      (left, right) =>
        left.replacement.start - right.replacement.start ||
        left.replacement.end - right.replacement.end,
    );
  const result: Array<number | undefined> = new Array(replacements.length);
  let previousEnd = 0;
  let delta = 0;
  for (const { replacement, index } of ordered) {
    if (
      replacement.start < previousEnd ||
      replacement.end < replacement.start ||
      replacement.cursorOffset < 0 ||
      replacement.cursorOffset > replacement.text.length
    ) {
      throw new Error("invalid or overlapping text replacements");
    }
    result[index] = replacement.start + delta + replacement.cursorOffset;
    delta += replacement.text.length - (replacement.end - replacement.start);
    previousEnd = replacement.end;
  }
  return result.map((offset) => {
    if (offset === undefined) {
      throw new Error("missing cursor offset");
    }
    return offset;
  });
}

export class AsyncGeneration {
  #current = 0;
  #disposed = false;

  begin(): number {
    return ++this.#current;
  }

  isCurrent(generation: number): boolean {
    return !this.#disposed && generation === this.#current;
  }

  invalidate(): void {
    this.#current++;
  }

  dispose(): void {
    this.#disposed = true;
    this.#current++;
  }
}
