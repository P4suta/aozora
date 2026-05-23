/**
 * 開発時にだけ stderr / stdout に出す軽量ロガー。
 *
 * production bundle では `import.meta.env.DEV` が `false` に解決されて
 * `console.error` / `console.warn` の呼び出し自体が剥がれる（Vite の
 * tree-shaking）。ユーザーには見えないところで握りつぶされる。
 *
 * 例外があった時にユーザーに伝えたい場合は呼び出し側で toast を出すこと。
 * このロガーは「デバッグ目的でしか出さない」もの専用。
 */

export function error(...args: unknown[]): void {
  if (import.meta.env.DEV) {
    // eslint-disable-next-line no-console
    console.error(...args);
  }
}

export function warn(...args: unknown[]): void {
  if (import.meta.env.DEV) {
    // eslint-disable-next-line no-console
    console.warn(...args);
  }
}

export function info(...args: unknown[]): void {
  if (import.meta.env.DEV) {
    // eslint-disable-next-line no-console
    console.info(...args);
  }
}
