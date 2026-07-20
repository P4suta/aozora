/**
 * Vitest 用の aozora-wasm スタブ。テストでは Document クラスや
 * `slugs` を実際には呼ばない（純粋関数のみテスト対象）が、
 * 解決対象の import を成功させるためにダミーを export する。
 */

export class Document {
  constructor(_source: string) {
    /* no-op */
  }
  toHtml(): string {
    return '';
  }
  toSource(): string {
    return '';
  }
  edit(_edits: unknown[]): void {}
  nodes(): unknown[] {
    return [];
  }
  diagnostics(): unknown[] {
    return [];
  }
  pairs(): unknown[] {
    return [];
  }
  gaiji(): unknown[] {
    return [];
  }
  gaijiAt(_byte_offset: number): undefined {
    return undefined;
  }
  sourceByteLen(): number {
    return 0;
  }
  free(): void {
    /* no-op */
  }
}

export function slugs(): unknown[] {
  return [];
}

export function version(): string {
  return '0.0.0-test';
}

export function prewarm(): void {}

export default async function init(): Promise<void> {
  /* no-op */
}
