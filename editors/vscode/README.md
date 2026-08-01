# aozora — VS Code extension for aozora-flavored markdown

青空文庫記法 (`.afm` / `.aozora` / `.aozora.txt`) を VS Code で書くための
拡張機能。言語サーバは
[`aozora`](https://github.com/P4suta/aozora/tree/main/crates/aozora-cli)
CLI に組み込まれた `aozora lsp` サブコマンド (in-process) で、そのクライアント
として診断・フォーマット・補完・プレビューをサーバに委ねる。
コマンドは Command Palette の `Aozora:` プレフィクスから引ける。

## インストール

VS Code Marketplace または Open VSX から `aozora-vscode` をインストールする。
コマンドラインでは:

```sh
code --install-extension yasunobu-sakashita.aozora-vscode
```

対象プラットフォーム向け `.vsix` を取得した場合は
`code --install-extension FILE.vsix` でもインストールできる。

各プラットフォーム向けパッケージには言語サーバ兼 CLI が同梱されるため、追加設定
なしで診断・補完・フォーマット・プレビュー・workspace lint が動作する。独自ビルド
を使う場合だけ `aozora.lsp.path` または `aozora.cli.path` を設定する
（`${workspaceFolder}` が使える）。

ソースから開発する場合は `editors/vscode/` で `bun install` を実行し、VS Code
から F5 を押す。

## ライセンス

Apache-2.0 OR MIT, at your option.

## ソース・バグ報告

[github.com/P4suta/aozora](https://github.com/P4suta/aozora) — issues / PR 歓迎。
