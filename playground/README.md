# Aozora Playground

Interactive web playground for the Aozora notation parser. Solid + Vite + WebAssembly + CodeMirror 6.

Live: <https://p4suta.github.io/aozora/playground/>

## エディタ機能

ブラウザ上の CodeMirror 6 エディタに、リポジトリ内の VSCode 拡張機能（`editors/vscode/`）から
主要な編集体験を移植しています。

- **シンタックスハイライト** — `nodes_json` から ruby / bouten / gaiji / 注記 / 見出し / 改ページなどを色分け
- **リアルタイム診断** — 括弧の未閉鎖 / 孤立 close / PUA 文字混入を CM6 linter で赤波線・警告表示
- **スラグ補完** — `［＃` を打つと spec の SLUGS カタログから候補を表示
- **構造化スニペット** — `#`, `｜`, `《`, `※` の即時テンプレート展開
- **外字ホバー** — `※［＃...］` 上にマウスを置くと解決結果（→ 字 + Unicode）を表示
- **外字インレイヒント** — gaiji span の直後に `→解決字` を inline で表示
- **連動編集** — `《》`, `「」`, `〔〕`, `［］` の片方を消すと反対側も消える
- **全角括弧の自動閉じ** — `《`, `「`, `〔`, `（`, `［` の入力で対応する閉じが挿入される
- **placeholder** — 空のエディタには「青空文庫記法を入力…」のヒント
- **折りたたみ** — `［＃ここから...］...［＃ここで...終わり］` ブロックを fold
- **見出しアウトライン** — 大/中/小の階層付きで右ペインの Outline タブから一覧 + ジャンプ
- **半角→全角即時変換** — `[ → ［`, `| → ｜`, `* → ※` など 8 文字を入力時に変換（IME 中は抑制）
- **ラップコマンド** — 選択 + `Ctrl/Cmd+Alt+R`（ルビ）、`Ctrl/Cmd+Alt+B`（傍点）等
- **性能プロファイル** — PerfBadge をクリックでメソッド別レイテンシ（parse / to_html / serialize / 各 JSON / gaiji）を popover 表示
- **記法ガイド** — `📖 記法ガイド` ボタンで完全リファレンスをモーダル表示、左カラムに目次つき、Esc / クリック外で閉じる
- **縦書きプレビュー** — HTML プレビューで縦書き / 横書きを切替（縦書き時は明朝体 + line-height 拡大）
- **テーマ切替** — Auto / Light / Dark の 3 段階。OS 設定追従または強制
- **タブ永続化** — Preview Pane のアクティブタブをリロード後も復元
- **localStorage 永続化** — エディタ内容を自動保存し、リロード後に復元（共有 URL とは独立）
- **共有 URL** — `?text=` で Base64url + UTF-8 エンコードしたテキストをコピー / 復元
- **設定パネル** — 半角→全角 / inlay hints / テーマ / 保存リセットを `⚙` から
- **モバイル対応** — レスポンシブレイアウト、layout-mode トグル（エディタのみ / 分割 / プレビューのみ）
- **アクセシビリティ** — モーダル focus trap、`role="dialog"`、toast `aria-live`、44px タッチターゲット

## Local development

このプロジェクトは [Bun](https://bun.com) を使います（lockfile は `bun.lock` テキスト形式）。

### 推奨：Docker（host 環境を汚さない）

aozora ルートの `Dockerfile` / `docker-compose.yml` が dev container に Rust /
wasm-pack / bun などすべてを内包しています。host には `docker` と `docker compose`
さえあれば動きます。

```sh
# 1. WASM crate をビルド（aozora ルートで）
docker compose run --rm dev wasm-pack build --target web --release crates/aozora-wasm

# 2. playground 依存をインストール
docker compose run --rm playground bun install

# 3. dev server を起動
docker compose up playground
# → http://localhost:5173/aozora/playground/
```

VS Code Dev Containers を使う場合、aozora ルートの `.devcontainer/devcontainer.json`
が `dev` service にアタッチします。コンテナ内で `cd playground && bun run dev --host` で
同じ dev server が立ち上がります。

### ホストの bun を直接使う場合

`bun` と `wasm-pack` と `rustup target add wasm32-unknown-unknown` を host に
インストール済みなら以下でも動きます。

```sh
wasm-pack build --target web --release ../crates/aozora-wasm
bun install
bun run dev
```

The Vite alias `aozora-wasm` resolves to `../crates/aozora-wasm/pkg/aozora_wasm.js`,
so re-running `wasm-pack build` (after editing the Rust side) and reloading the
browser is enough to pick up changes.

## Build

```sh
# Docker
docker compose run --rm playground bun run build

# または host で
bun run build
# → playground/dist/  (deployed to https://p4suta.github.io/aozora/playground/)
```

The `base: '/aozora/playground/'` in `vite.config.ts` makes asset URLs match the
GitHub Pages subpath. A strict Content-Security-Policy meta tag (`PROD_CSP` in
`vite.config.ts`) is injected into the production build only — defense-in-depth
over the renderer's escaping for the `innerHTML`-mounted preview. It is
build-time-only so the dev HMR WebSocket keeps working in `bun run dev` (a
`<meta>` CSP cannot be relaxed per-environment).

## アーキテクチャ要点

- `src/editor/parserState.ts` — `aozora-wasm` の `Document` を所有する CM6 `StateField`。
  source 変更で `to_html` / `serialize` / 各 JSON を**一度だけ**呼び、結果と parsed 配列
  （`nodes / diagnostics / pairs / gaijiResolutions`）をキャッシュ。前世代の Document は
  `.free()` する。UTF-8 ↔ UTF-16 オフセット変換テーブル（`u2b` / `b2u`）と、見出し階層・
  container fold range もここで構築。
- `src/editor/decorations.ts` — `nodes` を viewport 範囲だけ二分探索（`utils.ts` の
  `lowerBoundByStart`）で切り出し、`RangeSetBuilder` で `Decoration.mark` を構築。
- `src/editor/{linter,completion,hover,inlayHints,linkedRanges,folding,onType,wrapCommands}.ts` —
  各機能を独立した CM6 extension として実装。`parserStateField` から共通データを参照、
  自前で JSON.parse はしない。
- `src/editor/index.ts` — `createAozoraEditor` ファサード。extension array の組立、
  `halfWidthCompartment` / `inlayHintsCompartment` 経由の動的設定切替、`externalUpdate`
  annotation による外部 setSource の echo 抑止。
- `src/theme.ts` — `<html data-theme>` の bootstrap。OS 設定 + ユーザー preference の
  解決と、`prefers-color-scheme` 変化への追従。
- `src/storage.ts` — localStorage 薄ラッパ。失敗時は `saveSource` が `boolean` を返し、
  呼び出し側でトースト通知。複数 key（source / active tab / theme）を `aozora-playground:`
  名前空間で扱う。

### `aozora-wasm` 拡張

Phase 2/3 で追加した API：

- `resolve_gaiji_at(byte_offset)` — カーソル位置の `※［＃...］` 解決（hover 用、512 byte 窓）
- `gaiji_resolutions_json()` — 全 gaiji 一括解決（inlay hints 用）
- `slugs_json()` — `aozora::SLUGS` カタログ全体（completion 用）
- `profile_json()` — parse / to_html / serialize / 各 JSON / gaiji の独立タイミング

### `profile_json` は 2 回計算する

`profile_json` は計測のために `Document.parse()` と各 method を**改めて 1 回ずつ呼ぶ**。
通常フロー（`parserState.ts` の computeParserState）で既に同じ method が呼ばれているので、
合計で 2 回計算することになる。これは意図的：

- 1 回目の中に JS / WASM 境界の overhead が混ざると、popover に出す ms 値が不正確になる
- 2 回目は Rust 側で `performance.now()` だけ挟んだ純粋な内部時間

WASM パースは MB/s オーダーで速いので、6 MB 級でも体感差はない（数十 ms）。

### テスト

`bun run test` で vitest を実行。純粋関数のみテスト対象：

- `src/__tests__/share.test.ts` — UTF-8 / 全角 / surrogate pair / 共有 URL 上限判定
- `src/__tests__/storage.test.ts` — localStorage の往復と key 名前空間
- `src/__tests__/utils.test.ts` — `lowerBoundByStart` の二分探索
- `src/__tests__/parserState.test.ts` — `buildOffsetTables` の不変条件（u2b 単調性、surrogate ペア計上）

CM6 / WASM 結合テストは別タスク（Playwright E2E）。
