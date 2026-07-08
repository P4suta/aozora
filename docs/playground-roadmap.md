# Playground ロードマップ

`https://p4suta.github.io/aozora/playground/` のロードマップ。Phase 1（基盤）と
Phase 2（CodeMirror 6 + VSCode 拡張機能の移植 + Docker 化）は完了済み。

このファイルは **設計判断・非目標の記録**（なぜ保留したか／何を意図的にやらないか）と、
実行可能な backlog（→ GitHub issue **#440–#444**）へのインデックス。優先度や〆切は付けない。

> かつての追跡アンカー issue #83 は 2026-07-08 に退役。実行可能な backlog は下記の
> focused issue へ昇格し、このドキュメントは設計判断と非目標の記録として存続する。

---

## Phase 2 で意図的に保留した検討事項

### OPEN QUESTION（実装時に判断保留） → #440 で追跡

各行の trigger が発火した時点で着手する。詳細な rationale は #440 に転記済み。

| ID | 概要 | 現状の判断 | 検討すべきタイミング |
|---|---|---|---|
| **S4-Q1** | `ruby` kind の span 形状（base + reading 全体 vs 分離） | 全体を 1 mark でハイライト | ruby base / reading の色を分けたくなった時に `nodes_json` を実機調査 |
| **S6-Q1** | `pairs_json` の sanitized-source coords と raw source coords の乖離 | PUA 含み入力は linked editing を実機で検証していない | PUA を含む入力で linked editing が破綻したという報告が来た時 |
| **S6-Q2** | 4 種括弧（`［］/《》/「」/〔〕`）すべてに linked editing するかルビ系除外か | 全 4 種に対応、ただし最小ペア（1文字）のみ | 「ルビの開き括弧消したら閉じ括弧消える挙動が邪魔」という報告が来た時 |
| **S9-Q1** | `pairs_json` が `containerOpen`/`containerClose` ペアも出すか | 出していないと仮定して `nodes_json` から stack で自前マッチ | `pairs_json` が container を出すようになった時に folding.ts を簡素化 |
| **S11-Q1** | 半角→全角変換の context awareness（スラグ内で `[` 抑制等） | コンテキスト判定なしの単純全変換 | スラグ内で意図的に `[` を残したいユーザー要求が来た時 |
| **S12-Q1** | wrap コマンドの un-wrap（既に wrapper 内なら剥がす）挙動 | un-wrap 未実装、常に追加 wrap | リポジトリ内の VSCode 拡張の挙動を実機で見比べて差分があれば追従 |

> 解決済み: **S12-Q2**（全角キー `「`・`〔`・`＃` のキーバインド）→ コマンドパレット #334 で解消。
> **S15-Q1**（記法ガイドの markdown レンダラ）→ `marked`（~10 KB gzip）採用で解消。bundle 削減が
> 課題化したら自前簡易レンダラに切替。

### 今回スコープ外として明示的に除外した選択肢

| 項目 | 除外理由 | 再評価のトリガー |
|---|---|---|
| **LSP の WASM 化** (`aozora-lsp` を web worker で動かす) | tower-lsp + tokio の WASM 化が重い。aozora-wasm の JSON API でほぼ同等のことができる | 「LSP 機能（formatting / code actions）が真に必要」となった時 |
| **tree-sitter-aozora を Lezer に移植** | aozora-wasm の `nodes_json` が source-keyed の正解を返すので二重実装になる | `nodes_json` の精度・粒度が足りないハイライト要求が出た時 |
| **aozora-fmt を WASM 化** （フォーマッタ） | コード量が大きく、`toSource()` で round-trip 整形済み | aozora-fmt 独自のフォーマット結果が toSource() と乖離する事例が出た時 |
| **Pandoc 出力タブ** | `aozora-pandoc` は workspace member だが WASM ビルド対象外、別 crate の WASM 化が必要 | Pandoc 連携ニーズの強い要望が出た時 |
| **VSCode 拡張側のコマンド全 13 個移植** | `preview.ts`, `outline.ts`, `notationGuide.ts` 等は webview/extension API 依存。Web playground の UI 文脈で再設計が必要 | 個別機能の要求がきた時に CM6 native で書き直し |

> 注: コマンドパレット（#334）は上記「VSCode 拡張コマンド移植」の最初の CM6 native 実例
> （パレット 1 個のみ・全 13 個移植は依然として非目標）。

---

## 将来の機能候補 → #441–#444 で追跡

実行可能な backlog は下記の focused issue へ昇格済み。各 issue に概要・トリガー・現状を転記済み。

- **#441** — 永続化・複数ドキュメント・設定共有: IndexedDB 永続化 / 複数ファイル管理 / 設定 URL 共有（`?vertical=/?inlay=`）/ 左右ペイン同期スクロール
- **#442** — 出力・エクスポート: LaTeX・ePub・PDF 出力 / Pandoc 経由 50+ フォーマット / Markdown・RST → Aozora 逆変換
- **#443** — 共同編集・履歴・解析: Yjs collab / 入力履歴・undo 永続化 / スナップショット AST diff / per-stage 性能プロファイル
- **#444** — UX・A11y 磨き込み + コーパス: モバイル/タッチ深掘り / A11y 深掘り（spoken preview・screen reader）/ カスタム CSS テーマ / 青空文庫からの作品 import / 長文プリセット拡充

> 実装済み: **E2E（Playwright）** #335 ・ **i18n（英語 UI）** #336 ・ **og:image / og:description** ・
> **gzip share URL** #319 ・ **コマンドパレット** #334（全角キー打鍵不能を解消）・
> **localStorage 永続化**（`storage.ts`・タブを閉じても source 復元）・
> **A11y 基盤**（aria-role / focus-trap — `App.tsx`・`CommandPalette.tsx` 等）・
> **性能プロファイル**（`PerfBadge.tsx` の per-method 計測）。
> A11y の spoken/screen-reader 深掘りは #444、per-stage（sanitize→tokenize→pair→classify）
> 内訳の計測は #443 に残課題として計上。
