# aozora — VS Code extension for aozora-flavored markdown

青空文庫記法 (`.afm` / `.aozora` / `.aozora.txt`) を VS Code で書くための
拡張機能。フォーマッタ、診断、外字ホバー、ルビ／傍点ラッパー、
プレビューが入っている。LSP サーバ
[`aozora-lsp`](https://github.com/P4suta/aozora/tree/main/crates/aozora-lsp)
のクライアント。

## できること

- **構文エラー診断** — 括弧不一致、閉じてないルビ、未知の `［＃…］`
  注記が Problems パネルに出る
- **フォーマッタ** — `Format Document` (Shift+Alt+F) でバッファを
  正規化された青空文庫記法に書き直す (`parse ∘ serialize`)
- **外字ホバー** — `※［＃「...」、mencode］` にカーソルを当てると
  解決された Unicode 文字と説明が出る
- **外字折りたたみ** — `※[#…]` 注記を解決字 1 文字に
  視覚的に折りたたむ(カーソルが上に来ると元の表記に戻る)
- **ルビ／傍点ラッパー** — テキスト選択 → 右クリ →
  「Aozora: 選択を括弧で囲む」 でルビ・二重ルビ・傍点・鉤括弧・
  亀甲・注記の 7 種類でラップ可能
  - `Ctrl+Alt+R` でルビ、`Ctrl+Alt+B` で傍点 のキーバインドあり
- **半角キーで全角入力** — `[#` → `［＃canonical］`、`<<` → `《》`、
  `|` → `｜` のスニペット展開(slug カタログ補完つき)
- **HTML プレビュー** — `Aozora: Open Preview` で横にプレビューを
  開き、編集にリアルタイム追従。既定は縦書き (青空文庫の本来の方向)、
  `Aozora: プレビューの縦書き／横書きを切り替え` で切り替え可能
- **アウトライン** — 見出しの一覧を Quick Pick で表示してジャンプ
- **記法ガイド** — `Aozora: 記法ガイドを開く` で完全リファレンスを
  ブラウザで開く

## インストール

VS Code Marketplace から `aozora` で検索 → Install。**LSP サーバ
バイナリ (`aozora-lsp`) は同梱されている**ので、Rust toolchain も
別途インストールも不要、入れた瞬間から動く。

対応プラットフォーム(プラットフォーム別 `.vsix` を Marketplace が
自動で配信):

- Linux x64 / arm64 (glibc / musl)
- macOS x64 / arm64
- Windows x64 / arm64

### 自分でビルドした `aozora-lsp` を使いたい場合

開発中に同梱バイナリを上書きしたいときは、`aozora.lsp.path` 設定で
パスを指定する:

```jsonc
{
  "aozora.lsp.path": "/absolute/path/to/your/aozora-lsp"
}
```

設定が空または `"aozora-lsp"`(デフォルト)のときは同梱バイナリ →
PATH 上の `aozora-lsp` の順で解決される。

## ファイルの関連付け

以下の拡張子は自動的に aozora 言語モードになる:

- `.afm`
- `.aozora`
- `.aozora.txt`

通常の `.txt` は触らないが、ファイル先頭に青空文庫記法の特徴的な
マーカー(`［＃`、`｜X《`、ヘッダ区切り線)が含まれていれば
`aozora.autoDetect.plaintext` が自動で aozora モードに切り替える
(設定で無効化可能)。

`.txt` で auto-detect がかからないファイルは、
`Ctrl+K M` → "Aozora" で手動で言語モードを切り替える。

## 設定

| キー | デフォルト | 説明 |
|---|---|---|
| `aozora.lsp.path` | `aozora-lsp` | `aozora-lsp` 実行ファイルへのパス。`PATH` 上にあればこのままで OK |
| `aozora.trace.server` | `off` | LSP メッセージのトレース (`off` / `messages` / `verbose`) |
| `aozora.autoDetect.plaintext` | `true` | `.txt` ファイルを開くとき青空文庫記法を検出して自動でモード切替 |
| `aozora.gaijiFold.enabled` | `true` | `※［＃…］` 外字注記を解決字 1 文字に視覚折りたたみ |
| `aozora.preview.writingMode` | `vertical` | プレビュー WebView の組み方向 (`vertical` / `horizontal`)。`Aozora: プレビューの縦書き／横書きを切り替え` で一時切替も可 |

## はじめての方へ

`Aozora — はじめての青空文庫記法` ウォークスルーを用意してある。
コマンドパレットから `Welcome: Open Walkthrough` → `Aozora` を
選ぶとガイド付きでひと通りの機能を試せる。

## ライセンス

Apache-2.0 OR MIT.

## ソース・バグ報告

[github.com/P4suta/aozora](https://github.com/P4suta/aozora) — issues / PR 歓迎。
