export class Document {
  readonly #source: string;

  constructor(source: string) {
    this.#source = source;
  }

  toHtml(): string {
    const escaped = this.#source
      .replaceAll('&', '&amp;')
      .replaceAll('<', '&lt;')
      .replaceAll('>', '&gt;');
    return `<p>${escaped}</p>`;
  }

  toSource(): string {
    return this.#source;
  }

  edit(_edits: unknown[]): void {}

  nodes(): Array<{ kind: string; span: { start: number; end: number } }> {
    return [];
  }

  diagnostics(): Array<{
    kind: string;
    severity: 'error';
    source: 'source';
    span: { start: number; end: number };
  }> {
    const offset = new TextEncoder().encode(
      this.#source.slice(0, this.#source.indexOf('》')),
    ).length;
    return this.#source.includes('》')
      ? [
          {
            kind: 'unmatched_close',
            severity: 'error',
            source: 'source',
            span: { start: offset, end: offset + 3 },
          },
        ]
      : [];
  }

  pairs(): unknown[] {
    return [];
  }

  containerPairs(): unknown[] {
    return [];
  }

  gaiji(): unknown[] {
    return [];
  }

  gaijiAt(_byteOffset: number): undefined {
    return undefined;
  }

  sourceByteLen(): number {
    return new TextEncoder().encode(this.#source).length;
  }

  free(): void {}
}

export function slugs(): unknown[] {
  return [];
}

export function version(): string {
  return '0.0.0-test';
}

export function prewarm(): void {}

export default async function init(): Promise<void> {}
