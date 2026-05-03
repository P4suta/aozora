# aozora

<p align="center">
  <a href="https://github.com/P4suta/aozora/actions/workflows/ci.yml"><img alt="ci" src="https://github.com/P4suta/aozora/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/P4suta/aozora/actions/workflows/docs.yml"><img alt="docs deploy" src="https://github.com/P4suta/aozora/actions/workflows/docs.yml/badge.svg"></a>
  <a href="https://github.com/P4suta/aozora/releases/latest"><img alt="latest release" src="https://img.shields.io/github/v/release/P4suta/aozora?display_name=tag&sort=semver"></a>
  <a href="./LICENSE-APACHE"><img alt="license" src="https://img.shields.io/badge/license-Apache--2.0%20OR%20MIT-blue"></a>
  <a href="./rust-toolchain.toml"><img alt="msrv" src="https://img.shields.io/badge/rust-1.95-orange"></a>
</p>

<p align="center">
  📚 <a href="https://p4suta.github.io/aozora/"><strong>ハンドブック (mdbook)</strong></a>
  · 📖 <a href="https://p4suta.github.io/aozora/api/aozora/"><strong>API リファレンス (rustdoc)</strong></a>
  · 📦 <a href="https://github.com/P4suta/aozora/releases"><strong>リリース・バイナリ</strong></a>
  · 🇬🇧 <a href="./README.md"><strong>English</strong></a>
</p>

**青空文庫記法** を解析する純粋関数型 Rust パーサ。
ルビ (`｜青梅《おうめ》`)、傍点 (`［＃「X」に傍点］`)、縦中横、
外字参照 (`※［＃…、第3水準1-85-54］`)、訓点・返り点、
字下げ・地寄せコンテナ
(`［＃ここから2字下げ］… ［＃ここで字下げ終わり］`)、
改ページ・改段に対応します。

このパーサは **CommonMark / Markdown を一切扱いません** ―― 純粋に
青空文庫記法のみを対象としています。レンダラはセマンティックな
HTML5 を出力し、レクサは構造化された診断情報を返します。AST は
借用アリーナ上のツリーで、ソースバイトをコピーせず O(n) で走査
できます。

## インストール

### ビルド済み CLI バイナリ

`aozora` CLI のビルド済みバイナリは **Linux x86_64** / **macOS arm64** /
**Windows x86_64** の3プラットフォーム向けに、毎リリースで
[GitHub Releases](https://github.com/P4suta/aozora/releases) に
添付されます。`aozora-vX.Y.Z-<target>.{tar.gz,zip}` 形式のアーカイブと
`SHA256SUMS` がセットで配布されます。

### ソースからビルド

```sh
cargo install --git https://github.com/P4suta/aozora --locked aozora-cli
```

(`main` の最新から build します。再現性が必要なら release tag に pin する形が
[install 章](https://p4suta.github.io/aozora/getting-started/install.html) にあります。)

### Rust ライブラリとして利用

`Cargo.toml` のスニペット (現行リリース tag 入り) は
[install 章](https://p4suta.github.io/aozora/getting-started/install.html#as-a-rust-library)
に集約しています — 複数の README に tag を分散させるとリリース毎に書き換え
漏れが出るため、handbook の 1 箇所だけで管理する形にしています。crates.io
公開は v1.0 API 確定後の予定です。

WASM / C ABI / Python バインディングについては
[ハンドブックの Bindings 章](https://p4suta.github.io/aozora/bindings/rust.html)
を参照してください。

## クイックスタート

```rust
use aozora::Document;

let source = "｜青梅《おうめ》".to_owned();
let doc = Document::new(source);
let tree = doc.parse();

let html: String = tree.to_html();
let canonical: String = tree.serialize();
let diagnostics = tree.diagnostics();

assert_eq!(canonical, "｜青梅《おうめ》");
```

`Document` は [`bumpalo`](https://docs.rs/bumpalo) アリーナを所有し、
`tree` はそのアリーナから借用します。`Document` を drop すると
`Bump::reset` ひとつでツリー全体が解放されます。

## CLI

```sh
aozora check FILE.txt           # 字句解析・診断を出力
aozora fmt --check FILE.txt     # parse ∘ serialize の往復チェック
aozora render FILE.txt          # HTML を標準出力へ
aozora check -E sjis FILE.txt   # Shift_JIS ソース (青空文庫の標準)
```

すべてのサブコマンドは `-` (またはパス省略) で標準入力から読み込めます。
詳細は [CLI リファレンス章](https://p4suta.github.io/aozora/ref/cli.html)
を参照してください。

## クレート構成

aozora は18クレートの workspace です。
[`crates/aozora`](./crates/aozora) が公開ファサードで、ライブラリ
利用者は通常このひとつだけインポートします。

| クレート | 役割 |
|---|---|
| [`crates/aozora`](./crates/aozora) | 公開ファサード。`Document::parse() → AozoraTree<'_>` と `Diagnostic` 型、`SLUGS` カタログを提供。 |
| [`crates/aozora-spec`](./crates/aozora-spec) | 共有型の単一の出所: `Span`, `TriggerKind`, `PairKind`, `Diagnostic`, PUA センチネル。内部依存なし。 |
| [`crates/aozora-syntax`](./crates/aozora-syntax) | AST ノード型 (`AozoraNode` 借用アリーナ variants, `ContainerKind`, `BoutenKind`, `Indent`)。 |
| [`crates/aozora-encoding`](./crates/aozora-encoding) | Shift_JIS デコード + 外字解決 (PHF テーブル、JIS X 0213 + UCS フォールバック)。 |
| [`crates/aozora-scan`](./crates/aozora-scan) | SIMD 対応マルチパターンスキャナ (Teddy / Hoehrmann-DFA / memchr フォールバック)。 |
| [`crates/aozora-veb`](./crates/aozora-veb) | Eytzinger 配置の sorted-set 検索 (キャッシュ親和的二分探索)。 |
| [`crates/aozora-lexer`](./crates/aozora-lexer) | 7-phase 字句解析パイプライン。 |
| [`crates/aozora-lex`](./crates/aozora-lex) | 融合ストリーミング・オーケストレータ ―― 純粋関数 `fn(&str) -> AozoraTree<'_>`。 |
| [`crates/aozora-render`](./crates/aozora-render) | HTML / canonical シリアライザ ―― `html::render_to_string`, `serialize::serialize`。 |
| [`crates/aozora-cli`](./crates/aozora-cli) | `aozora` バイナリ本体: `check` / `fmt` / `render`。 |
| [`crates/aozora-wasm`](./crates/aozora-wasm) | `wasm32-unknown-unknown` ターゲット (`wasm-pack build --target web`)。 |
| [`crates/aozora-ffi`](./crates/aozora-ffi) | C ABI ドライバ (オペーク・ハンドル + JSON 構造化データ)。 |
| [`crates/aozora-py`](./crates/aozora-py) | PyO3 バインディング、`maturin` で配布。 |
| [`crates/aozora-bench`](./crates/aozora-bench) | Criterion + コーパス駆動プローブ (PGO トレーニング元)。 |
| [`crates/aozora-corpus`](./crates/aozora-corpus) | コーパス抽象化 (sweep テスト用、dev 限定。`AOZORA_CORPUS_ROOT` で参照)。 |
| [`crates/aozora-test-utils`](./crates/aozora-test-utils) | proptest 用ストラテジ共有 (dev 限定)。 |
| [`crates/aozora-trace`](./crates/aozora-trace) | samply トレース用 DWARF シンボリケータ。 |
| [`crates/aozora-xtask`](./crates/aozora-xtask) | リポジトリ自動化 (samply ラッパ、トレース解析、コーパス pack/unpack)。 |

クレート間の階層構造の詳細は
[Architecture → Crate map](https://p4suta.github.io/aozora/arch/crates.html)
を参照してください。

## 開発

すべての操作は Docker 内で実行します ―― ホスト側の toolchain は
触りません。dev イメージを一度ビルドし、あとは `just` 経由で実行
します。

```sh
just                # ターゲット一覧
just build          # cargo build --workspace --all-targets
just test           # cargo nextest run --workspace
just prop           # property ベースのスイープ (block あたり 128 ケース)
just lint           # fmt + clippy pedantic+nursery + typos + strict-code
just deny           # cargo-deny licenses + advisories + bans
just coverage       # cargo llvm-cov による branch coverage
just ci             # CI パイプラインの完全レプリカ
just book-build     # mdbook ハンドブックをビルド
just book-serve     # localhost:3000 でハンドブックをライブプレビュー
```

CLI をコンテナ内で起動するには `just run` を使います:

```sh
just run check FILE.txt
just run render -E sjis FILE.txt > out.html
```

コントリビュートの詳細は
[CONTRIBUTING.md](./CONTRIBUTING.md) または
[ハンドブックの Contributing 章](https://p4suta.github.io/aozora/contrib/dev.html)
を参照してください。

## ドキュメント

- 📚 [**ハンドブック**](https://p4suta.github.io/aozora/) ―― mdbook による
  包括的なドキュメント (記法リファレンス、アーキテクチャ、性能、
  バインディング、コントリビュート)。
- 📖 [**API リファレンス**](https://p4suta.github.io/aozora/api/aozora/)
  ―― 自動デプロイされる rustdoc。
- [`CONTRIBUTING.md`](./CONTRIBUTING.md) ―― 開発フロー、TDD ポリシー、
  PR ルール。
- [`SECURITY.md`](./SECURITY.md) ―― 脆弱性報告窓口。
- [`CHANGELOG.md`](./CHANGELOG.md) ―― リリース履歴。

## 関連プロジェクト

| Repo | 概要 |
|---|---|
| [`P4suta/afm`](https://github.com/P4suta/afm) | CommonMark + GFM + 青空文庫記法 を統合した Markdown 方言。aozora を基盤として構築。 |
| [`P4suta/aozora-tools`](https://github.com/P4suta/aozora-tools) | オーサリングツール: フォーマッタ、LSP サーバ、tree-sitter 文法、VS Code 拡張。 |

## ライセンス

[Apache-2.0](./LICENSE-APACHE) または [MIT](./LICENSE-MIT) のデュアル
ライセンスです。利用者の選択に委ねられます (Rust コミュニティ慣例)。
サードパーティの著作権表示は [`NOTICE`](./NOTICE) を参照してください
(青空文庫の仕様スナップショット、テスト用パブリックドメイン作品)。
