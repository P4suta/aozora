# aozora — VS Code extension for aozora-flavored markdown

青空文庫記法 (`.afm` / `.aozora` / `.aozora.txt`) を VS Code で書くための
拡張機能。言語サーバ
[`aozora-lsp`](https://github.com/P4suta/aozora/tree/main/crates/aozora-lsp)
のクライアントで、診断・フォーマット・補完・プレビューはサーバが担う。
コマンドは Command Palette の `Aozora:` プレフィクスから引ける。

## インストール

**まだ公開していない。** Marketplace にも無く、`.vsix` も配布していない。
動かすにはリポジトリから:

```sh
cargo install --path crates/aozora-lsp   # 言語サーバ (~/.cargo/bin へ)
cd editors/vscode && bun install
```

`editors/vscode/` を VS Code で開いて F5 を押すと、拡張を読み込んだ
Extension Development Host が起動する。`.aozora` ファイルを開けば有効になる。

`aozora-lsp` は `PATH` から解決される。ビルドツリーのバイナリを直接使う
なら `aozora.lsp.path` に指定する (`${workspaceFolder}` が使える)。

## ライセンス

Apache-2.0 OR MIT, at your option.

## ソース・バグ報告

[github.com/P4suta/aozora](https://github.com/P4suta/aozora) — issues / PR 歓迎。
