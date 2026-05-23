/**
 * Editor 拡張で共有する小さなユーティリティ群。
 *
 * 現状の住人は `lowerBoundByStart` 1 個だけ。将来共通化したいヘルパ
 * （例：viewport クリッピング、span overlap 判定）が出てきたらここに
 * 集約する。
 */

interface HasSpanStart {
  span: { start: number };
}

/**
 * `entries` を `span.start` で並べた配列とみなし、`span.start < byte`
 * を満たす最後のエントリの「次の index」を返す二分探索。
 *
 * - decorations.ts で viewport に重なる decoration を切り出すのに使う
 * - 同じ shape の entries（NodeEntry / DiagnosticEntry / GaijiResolutionEntry）
 *   なら呼び出し側で start での昇順を保証していれば正しく動く
 */
export function lowerBoundByStart<T extends HasSpanStart>(entries: T[], byte: number): number {
  let lo = 0;
  let hi = entries.length;
  while (lo < hi) {
    const mid = (lo + hi) >>> 1;
    if (entries[mid]!.span.start < byte) lo = mid + 1;
    else hi = mid;
  }
  return lo;
}
