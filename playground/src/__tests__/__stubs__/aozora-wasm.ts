/**
 * Vitest 用の aozora-wasm スタブ。テストでは Document クラスや
 * `slugsJson` を実際には呼ばない（純粋関数のみテスト対象）が、
 * 解決対象の import を成功させるためにダミーを export する。
 */

export class Document {
  constructor(_source: string) {
    /* no-op */
  }
  toHtml(): string {
    return '';
  }
  serialize(): string {
    return '';
  }
  diagnosticsJson(): string {
    return '{"schemaVersion":1,"data":[]}';
  }
  nodesJson(): string {
    return '{"schemaVersion":1,"data":[]}';
  }
  pairsJson(): string {
    return '{"schemaVersion":1,"data":[]}';
  }
  gaijiJson(): string {
    return '{"schemaVersion":1,"data":[]}';
  }
  profileJson(): string {
    return '{"schemaVersion":1,"data":[]}';
  }
  resolveGaijiAt(_byte_offset: number): string {
    return 'null';
  }
  sourceByteLen(): number {
    return 0;
  }
  free(): void {
    /* no-op */
  }
}

export function slugsJson(): string {
  return '{"schemaVersion":1,"data":[]}';
}

export default async function init(): Promise<void> {
  /* no-op */
}
