# Playground ロードマップ

`https://p4suta.github.io/aozora/playground/` の今後の方向性メモ。Phase 1（基盤）と
Phase 2（CodeMirror 6 + aozora-tools 機能移植 + Docker 化）は完了済み。

このファイルは「いつ作る」ではなく「やるなら何を作る／なぜ未着手か」を残すための
インデックス。優先度や〆切は付けない（必要に応じて issue / project に昇格）。

---

## Phase 2 で意図的に保留した検討事項

### OPEN QUESTION（実装時に判断保留）

| ID | 概要 | 現状の判断 | 検討すべきタイミング |
|---|---|---|---|
| **S4-Q1** | `ruby` kind の span 形状（base + reading 全体 vs 分離） | 全体を 1 mark でハイライト | ruby base / reading の色を分けたくなった時に `nodes_json` を実機調査 |
| **S6-Q1** | `pairs_json` の sanitized-source coords と raw source coords の乖離 | PUA 含み入力は linked editing を実機で検証していない | PUA を含む入力で linked editing が破綻したという報告が来た時 |
| **S6-Q2** | 4 種括弧（`［］/《》/「」/〔〕`）すべてに linked editing するかルビ系除外か | 全 4 種に対応、ただし最小ペア（1文字）のみ | 「ルビの開き括弧消したら閉じ括弧消える挙動が邪魔」という報告が来た時 |
| **S7-Q1** | スラグカタログを TS ハードコード vs aozora-spec 連動 | aozora-wasm の `slugs_json()` で完全同期済み（実質解決） | aozora-spec で SLUGS の derive serde が入れば JSON export も検討 |
| **S9-Q1** | `pairs_json` が `containerOpen`/`containerClose` ペアも出すか | 出していないと仮定して `nodes_json` から stack で自前マッチ | `pairs_json` が container を出すようになった時に folding.ts を簡素化 |
| **S11-Q1** | 半角→全角変換の context awareness（スラグ内で `[` 抑制等） | コンテキスト判定なしの単純全変換 | スラグ内で意図的に `[` を残したいユーザー要求が来た時 |
| **S12-Q1** | wrap コマンドの un-wrap（既に wrapper 内なら剥がす）挙動 | un-wrap 未実装、常に追加 wrap | aozora-tools VSCode 拡張の挙動を実機で見比べて差分があれば追従 |
| **S12-Q2** | 全角キー（`「`, `〔`, `＃`）のキーバインド | コマンドパレットも未実装、ASCII の 3 キーのみバインド | 「コマンドパレット欲しい」報告が来た時 |
| **S13-Q1** | inlay hints のオン/オフ UI | SettingsPanel で実装済み（実質解決） | — |
| **S15-Q1** | 記法ガイドの markdown レンダラ | `marked` を採用済み（~10 KB gzip） | bundle 削減が課題になったら自前簡易レンダラに切替 |

### 今回スコープ外として明示的に除外した選択肢

| 項目 | 除外理由 | 再評価のトリガー |
|---|---|---|
| **LSP の WASM 化** (`aozora-lsp` を web worker で動かす) | tower-lsp + tokio の WASM 化が重い。aozora-wasm の JSON API でほぼ同等のことができる | 「LSP 機能（formatting / code actions）が真に必要」となった時 |
| **tree-sitter-aozora を Lezer に移植** | aozora-wasm の `nodes_json` が source-keyed の正解を返すので二重実装になる | `nodes_json` の精度・粒度が足りないハイライト要求が出た時 |
| **aozora-fmt を WASM 化** （フォーマッタ） | コード量が大きく、`serialize()` で round-trip 整形済み | aozora-fmt 独自のフォーマット結果が serialize() と乖離する事例が出た時 |
| **Pandoc 出力タブ** | `aozora-pandoc` は workspace member だが WASM ビルド対象外、別 crate の WASM 化が必要 | Pandoc 連携ニーズの強い要望が出た時 |
| **VSCode 拡張側のコマンド全 13 個移植** | `preview.ts`, `outline.ts`, `notationGuide.ts` 等は webview/extension API 依存。Web playground の UI 文脈で再設計が必要 | 個別機能の要求がきた時に CM6 native で書き直し |

---

## 将来の機能候補（順不同・スコープ未確定）

### 実用度を上げる

- **localStorage / IndexedDB 永続化** — タブを閉じても source が復元される。`?text=` 共有とは別レイヤ
- **モバイル最適化** — 現状 760px 切替の最小レスポンシブのみ。タッチでの折り畳み・タブ操作の改善余地大
- **長文用の gzip share URL** — `lz-string` 等で base64url + 圧縮、URL 長制限の緩和
- **複数ファイル管理** — ブラウザ内で複数 "ドキュメント" を Tab 切替、それぞれ別 `?text=` 共有
- **設定の URL 共有** — `?vertical=1&inlay=0` 等、エディタ設定もリンクで共有
- **左右ペインの同期スクロール** — エディタの可視範囲と preview の可視範囲を同期

### 機能の深さ

- **Yjs + CodeMirror collab で共同編集** — 教育・校正ワークフローに直結
- **入力履歴・undo の永続化** — エディタの履歴がブラウザリロード後も残る
- **スナップショット diff** — source 変更前後の AST diff を可視化
- **性能プロファイル可視化** — sanitize → events → pair → classify の各 phase の時間を表示

### 出力の拡張

- **LaTeX / ePub / PDF 出力** — フロント側で完結させるか、別 WASM ビルドが必要
- **Pandoc 経由の 50+ フォーマット** — `aozora-pandoc` を WASM 化したら可能
- **Markdown / RST → Aozora の逆変換** — 既存ドキュメントを青空文庫記法に移植する道具

### 開発・運用面

- **E2E テスト（Playwright）** — 13 機能すべてを自動テスト、CI で回す
- **i18n（英語 UI）** — UI と記法ガイドを英訳、海外開発者へのリーチ
- **A11y 強化** — spoken preview、focus order、screen reader 対応
- **og:image / og:description** — SNS シェア時のプレビューカード
- **カスタム CSS テーマ** — 横組み本 / 縦組み本 / モダン Web 風など preview の見た目を選択

### コーパス・サンプル

- **青空文庫からの作品 import** — ZIP URL 入力で .txt をフェッチ → Shift_JIS decode → 編集開始
- **代表作品のプリセット拡充** — 現在 9 サンプルだが、長文（章単位）も載せて速度を体感させる

---

## 「今すぐ次にやるなら」候補（実装者視点）

| 候補 | 規模 | 投資対効果 | コメント |
|---|---|---|---|
| **E2E テスト（Playwright）** | M（半日〜1 日） | ★★★ | エディタ + WASM の結合テスト。13 機能 + Docker + localStorage の品質保証、docs.yml の CI に組み込む |
| **i18n（英語 UI）** | M | ★★ | 海外向けリーチ、ただし日本語前提の記法なので効果は限定的 |
| **gzip share URL** | S | ★★ | `lz-string` 等で長文共有を可能に |
| **VSCode 拡張のコマンドパレット相当** | M | ★★ | 全角キー打鍵不能問題の解決。CM6 のシンプルな fuzzy menu で OK |
