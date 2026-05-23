/**
 * Vitest 用の aozora-wasm スタブ。テストでは Document クラスや
 * `slugs_json` を実際には呼ばない（純粋関数のみテスト対象）が、
 * 解決対象の import を成功させるためにダミーを export する。
 */

export class Document {
  constructor(_source: string) {
    /* no-op */
  }
  to_html(): string {
    return '';
  }
  serialize(): string {
    return '';
  }
  diagnostics_json(): string {
    return '{"schema_version":1,"data":[]}';
  }
  nodes_json(): string {
    return '{"schema_version":1,"data":[]}';
  }
  pairs_json(): string {
    return '{"schema_version":1,"data":[]}';
  }
  gaiji_resolutions_json(): string {
    return '{"schema_version":1,"data":[]}';
  }
  profile_json(): string {
    return '{"schema_version":1,"data":[]}';
  }
  resolve_gaiji_at(_byte_offset: number): string {
    return 'null';
  }
  source_byte_len(): number {
    return 0;
  }
  free(): void {
    /* no-op */
  }
}

export function slugs_json(): string {
  return '{"schema_version":1,"data":[]}';
}

export default async function init(): Promise<void> {
  /* no-op */
}
