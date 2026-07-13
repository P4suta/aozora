### 青空文庫 CLI のシェル文言 — 日本語。
###
### CLI 自身の人間向けの表層のみを localize する（stdin ガード、`--watch`
### バナー、`explain` のフッタ／節ラベル）。機械軸（json / short / コード /
### exit / schema / timing-json）は決してここを通さない。
### docs/adr/0033-cli-output-language-policy.md を参照。

## 入力ガード

# ドキュメント系サブコマンドが素の対話端末で stdin を読もうとしたとき（入力を
# 待って永久にブロックしてしまう）に表示する。$cmd はコピペ例に出るサブコマンド
# 名（例: "check" や "inspect nodes"）。
stdin-empty =
    error: 標準入力が空です (端末から実行中)
      ヒント: ファイルを →  aozora {$cmd} <FILE>
              パイプで   →  cat f.txt | aozora {$cmd}
      全機能:  aozora --help

## ウォッチモード

# `--watch` の再実行の合間に端末へ出すバナー。$path は監視中のファイル。
watch-banner = ── 監視中 {$path}（Ctrl-C で終了）──

## explain

# `aozora check` の人間向け診断のあとに出すフッタ。読者を
# `aozora explain <code>` へ誘導する。下に並ぶコード別のコマンド行はそのままの
# シェルコマンドで、localize しない。
explain-hint-header = ヒント: 詳細は `aozora explain <code>` を実行。例:
# フッタが列挙するより多くの別コードがあるときの末尾。$count は残り件数。
explain-hint-more = … 他 {$count} 件

# `aozora explain <code>` 出力内の節ラベル。周囲の診断プロース（タイトル／本文
# ／再現例／修正例）は spec 側の所有で、ここでは localize しない。
explain-repro-label = 再現例:
explain-fixed-label = 修正後:
explain-see-label = 参照:
