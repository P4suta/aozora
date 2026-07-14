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

## fmt バッチ UX

# ディレクトリ探索中に表示するスピナー（ファイル数は未確定）。
fmt-progress-discovering = ソースファイルを探索中…

# ディレクトリ fmt 実行後のバッチサマリ。$formatted は整形（--check / --list
# では整形対象）件数、$unchanged は変更なし件数、$errors は読み取り／整形に
# 失敗した件数。
fmt-summary = 整形 {$formatted} 件、変更なし {$unchanged} 件、エラー {$errors} 件

## explain

# `aozora check` の人間向け診断のあとに出すフッタ。読者を
# `aozora explain <code>` へ誘導する。下に並ぶコード別のコマンド行はそのままの
# シェルコマンドで、localize しない。
explain-hint-header = ヒント: 詳細は `aozora explain <code>` を実行。例:
# フッタが列挙するより多くの別コードがあるときの末尾。$count は残り件数。
explain-hint-more = … 他 {$count} 件

# `aozora explain <code>` 出力内の節ラベル。再現例／修正例そのものは
# aozora-spec 所有の言語中立な青空記法だが、localize された title／body の
# プロースは以下で診断コードをキーに定義する。
explain-repro-label = 再現例:
explain-fixed-label = 修正後:
explain-see-label = 参照:

# `aozora explain <TARGET>` が解決できない対象を指したときに出す（stderr・非ゼロ
# 終了）。$target は認識できなかった引数。
explain-unknown = 不明な explain 対象 `{$target}`
# 上の行に続けて、対象の近傍が見つかったとき（ノードタグ・概念・診断コードに
# 対する編集距離一致）に付ける。$suggestion は最も近い既知の対象で、localize
# しないリテラルな識別子。
explain-did-you-mean = もしかして `{$suggestion}`?
# 有効な集合の在処を示す末尾。`aozora kinds` と例のコードはそのままのシェル
# テキストで、どのロケールでも同じ。
explain-unknown-hint =
    NodeKind タグか記法概念（`aozora kinds` を実行）、または
    `aozora::lex::unclosed_bracket` のような診断コードを想定しています

## 記法概念
##
## `aozora explain <concept>` のプロース。読者が入力しがちだが NodeKind の
## ハンドブックページと 1 対 1 でない記法家系向け — 略語（`tcy`）や日本語名
## （`傍点`・`ルビ` …）。家系ごとに `concept-<slug>-title`（見出し）と
## `concept-<slug>-body`（短いプロース）を 1 組。織り込む青空記法のグリフは
## リテラルな構文で、どのロケールでも同じ。

concept-ruby-title = ルビ (ruby) — 読みの注記
concept-ruby-body =
    親文字に添える小さな読み: 青空《あおぞら》。範囲が曖昧なときは先頭の `｜` で
    親文字を明示する: ｜青空《あおぞら》。

    詳しくは `aozora explain ruby` でハンドブックページを参照。

concept-gaiji-title = 外字 (gaiji) — Unicode 外の文字参照
concept-gaiji-body =
    素の Unicode で書けない文字を ※［＃…］ 参照で表したもの — 多くは JIS X 0213 の
    面区点（`第3水準1-15-23`）や `U+XXXX` コード。

    詳しくは `aozora explain gaiji`。診断 `unresolved_gaiji` も参照。

concept-kaeriten-title = 返り点 (kaeriten) — 漢文の返り記号
concept-kaeriten-body =
    漢文を日本語の語順で読むための返り記号。［＃…］ 注記で書き、レ点や
    一/二・上/下・甲/乙 の各家系がある。

    詳しくは `aozora explain kaeriten`。診断 `bracketed_kaeriten_no_pair`・
    `kaeriten_outside_kanbun` も参照。

concept-bouten-title = 傍点 (bouten) — 強調の点
concept-bouten-body =
    語の傍らに打つ強調の点で、イタリックに相当する青空記法: ［＃「ここ」に傍点］。
    姉妹家系の 傍線 は点でなく線を引く。

    詳しくは `aozora explain bouten`。診断 `bouten_target_ambiguous` も参照。

concept-warichu-title = 割注 (warichu) — 行内の割り注
concept-warichu-body =
    本文の中に半分の高さで二行に組む注。［＃割り注］ で開き ［＃割り注終わり］ で
    閉じる。

    詳しくは `aozora explain warichu`。

concept-tcy-title = 縦中横 (tate-chu-yoko) — 縦組み中の横組み
concept-tcy-body =
    縦組みの中で短い横組み — たいてい二桁の数字 — を正立させる記法。
    ［＃「25」は縦中横］ と書く。

    NodeKind タグは `combineUpright`。診断 `tcy_target_not_found` も参照。

## 診断プロース
##
## 診断コードごとに `diag-<slug>-title`（見出し）と `diag-<slug>-body`（詳細）を
## 1 組ずつ持つ。<slug> はコードの末尾 `::` セグメントの `_` を `-` にしたもの
## （例: `aozora::lex::unclosed_bracket` → `unclosed-bracket`）。body の `{$…}`
## プレースホルダは消費側（CLI の explain・LSP のホバー）が実診断から埋める。
## このプロースは aozora-spec から剥離した — カタログ crate を純粋な機械契約に
## 保つため。コード・重大度・`#[error]` Display・JSON は決して localize しない。

diag-source-contains-pua-title = 私用領域文字がソースに紛れ込んでいる
diag-source-contains-pua-body =
    私用領域文字 `U+{$codepoint}` がソースに紛れ込んでいます。

    この文字 (`{$char}`) は青空文庫の通常テキストには現れない予約コードポイントで、aozora-lex の内部マーカー (U+E001..U+E004) と衝突します。
    通常はテキストエディタの非表示文字設定や、コピペ時の不可視文字で混入します。

    直し方: 該当の 1 文字を削除してください。

diag-unclosed-bracket-title = 閉じられていない開き括弧
diag-unclosed-bracket-body =
    閉じられていない `{$open}` があります。

    どこかに対応する `{$close}` を必ず置いてください。aozora 記法では一行内で閉じるのが基本です。

    例: {$example}

diag-unmatched-close-title = 対応する開き括弧のない閉じ括弧
diag-unmatched-close-body =
    対応する `{$open}` のない `{$close}` です。

    考えられる原因:
    1. 余分な `{$close}` を打ってしまった → 削除する
    2. 前にあるはずの `{$open}` が欠けている → 適切な位置に追加する
    3. その間に別の `{$close}` があり、ペアが一段ずれた → 該当箇所のペアを見直す

diag-accent-decomposition-applied-title = アクセント分解が適用された（情報）
diag-accent-decomposition-applied-body =
    「〔…〕」のアクセント表記が、サニタイズ段階で合成済み Unicode 文字へ分解されました（例: 〔e'〕→é）。

    これは意図された挙動（ADR-0003）で、情報提供のための Note です。

    直し方: 対応は不要です。保存（serialize）すると元の 〔…〕 形へ復元され、変換は無損失です。

diag-unresolved-gaiji-title = 外字参照が解決できなかった
diag-unresolved-gaiji-body =
    外字参照（※［＃…］）が Unicode 文字にも JIS X 0213 の面区点にも解決できませんでした。

    このため描画では意図した字形ではなく、説明テキストがそのまま表示されます。

    直し方: 参照に解決可能な指定を与えてください — `第3水準1-15-23` のような面区点、または `U+XXXX` 形式の Unicode 参照を補います。

diag-mismatched-container-close-title = 開いた種別と違う閉じで閉じたコンテナ
diag-mismatched-container-close-body =
    コンテナを `{$open_kind}` として開いたのに、`{$close_kind}` の閉じ指示で閉じています。

    開きと閉じの家系が食い違うため、範囲が正しく確定しません。

    直し方: 開いた家系に合わせて閉じてください — `ここから字下げ` は `ここで字下げ終わり`、`ここから地付き` は `ここで地付き終わり` のように対応させます。

diag-empty-ruby-reading-title = ルビの読みが空
diag-empty-ruby-reading-body =
    ベース付きルビ（｜ベース《…》）でベースはあるのに読みが空です。

    `｜` がある以上これは素の《》ではなく入力の書き損じで、ルビはプレーンテキストに退化します。

    直し方: 読みを補う（｜青空《あおぞら》）か、ルビをやめるなら ｜…《》 のマーカーごと外してベースを地の文にします。

diag-nested-ruby-title = ルビの読みの中で入れ子になったルビ
diag-nested-ruby-body =
    ルビの読みの中で、さらに別のルビ（《…》）が開かれています。

    ルビは入れ子にできないため、内側の《…》が問題箇所です。外側のルビは可能な範囲で解釈されます。

    直し方: 内側の《 の前で外側の読みを閉じるか、内側の《…》を取り除いてください。

diag-unrecognised-container-directive-title = 未知のコンテナ指示
diag-unrecognised-container-directive-body =
    `［＃ここから…］` はコンテナの開きに見えますが、既知のコンテナ名（字下げ／地付き／地から N 字上げ など）に一致しません。

    出力は保たれますが、コンテナとしては扱われず、ただの注記として残ります。

    直し方: 既知のコンテナ名に直してください（例: ［＃ここから2字下げ］）。

diag-tcy-target-not-found-title = 縦中横の対象が前方に見つからない
diag-tcy-target-not-found-body =
    縦中横の前方参照（［＃「X」は縦中横］）が指す対象 X が、直前までの本文のどこにも現れません。

    装飾すべき文字列が無いため、指示は Unknown 注記に退化します。

    直し方: 対象は注記より前の同じ行に現れている必要があります。綴りを確認するか、装飾したい文字列の後ろに ［＃「X」は縦中横］ を置いてください。

diag-bouten-target-ambiguous-title = 傍点の対象が複数あり曖昧
diag-bouten-target-ambiguous-body =
    傍点の前方参照（［＃「X」に傍点］）の対象 X が、直前までに複数回現れています。

    どの出現に傍点を付けるか一意でないため、意図しない箇所が装飾されるおそれがあります（パーサは look-back 規則で1つに決めます）。

    直し方: 対象が一意になるよう言い換えてください（例:「白い花」のように限定する）。

diag-forward-referent-not-stylable-title = 前方参照の対象がその場で装飾できない
diag-forward-referent-not-stylable-body =
    前方参照の対象 X は直前までに存在しますが、その場で装飾できません — ルビのベース、前の行、別の構造の内側、または複数候補のいずれかです。

    注記は保持され本文は往復しますが、装飾は前の出現には適用されません。

    直し方: 対象がプレーンに現れる箇所の隣へ ［＃…］ を移動してください。

diag-break-in-single-line-container-title = 単一行コンテナ内の改ページ／改段
diag-break-in-single-line-container-body =
    単一行コンテナ（`{$container}`）と同じ行に、改ページ／改段が現れました。

    単一行コンテナはその行の残りだけに効くため、行内の改行系指示はコンテナの効果を落とします。

    直し方: 改ページを行外へ出すか、改行をまたいで効く ［＃ここから…］ … ［＃ここで…終わり］ のブロック形式を使ってください。

diag-bracketed-kaeriten-no-pair-title = 対応する基点のない角括弧返り点
diag-bracketed-kaeriten-no-pair-body =
    角括弧返り点（［＃二］／［＃下］／［＃乙］ など）に対応する家系の基点（［＃一］／［＃上］／［＃甲］）が、文書中のどこにもありません。

    返るべき先が無いため、返り点として成立しません。

    直し方: 家系の基点を文書のどこかに置いてください — ［＃二］/［＃三］には ［＃一］、［＃下］/［＃中］には ［＃上］、［＃乙］…には ［＃甲］。

diag-kaeriten-outside-kanbun-title = 漢文文脈の外に現れた返り点
diag-kaeriten-outside-kanbun-body =
    返り点（［＃二］／［＃レ］ など）が漢文的でない文脈に現れています — 文書中で唯一の返り点で、周囲が普通のかな文です。

    本物の返り点ではなく、紛れ込んだ注記の可能性が高いと判定されました。

    直し方: 本物の返り点なら漢文文脈で使い、そうでなければ該当の ［＃…］ 注記を削除してください。

diag-mismatched-bouten-container-title = 傍点と傍線で開閉が食い違うレンジ
diag-mismatched-bouten-container-body =
    傍点／傍線のレンジを `{$open_family}` で開いたのに、`{$close_family}` の閉じで閉じています。

    点と線は描画が異なるため、その範囲の強調が曖昧になります（パーサは開き側の家系で復旧します）。

    直し方: 開いた家系に合わせて閉じてください — 傍点は ［＃傍点終わり］、傍線は ［＃傍線終わり］。

diag-non-canonical-directive-title = 非正規の綴りの ［＃…］ 注記
diag-non-canonical-directive-body =
    非正規の綴りの ［＃…］ 注記です。正規形は `［＃{$canonical}］` です。

    この注記の中身は、登録済みの記法を非正規な綴り（送り仮名・同義語・綴りゆれ）で書いたものと判定され、Unknown 注記のまま保持されています。パーサは中身を書き換えません。

    直し方: `［＃{$canonical}］` に書き換えてください。`aozora fmt --fix` で自動修正できます。

diag-residual-annotation-marker-title = 未分類の ［＃…］ 注記（パイプライン内部）
diag-residual-annotation-marker-body =
    未分類の ［＃...］ 注記です。

    注記辞典 (gaiji_chuki) のキーワードに合致しなかったか、誤字の可能性があります。

    確認手順:
    1. ［＃ の中身が `改ページ` / `中央揃え` などの登録済みキーワードと一致するか確認
    2. `第3水準1-...` のような JIS X 0213 面区点コードを付け忘れていないか確認
    3. それでも不明な場合は説明のみ形式 (※［＃「説明」］) でひとまず通せます

diag-unregistered-sentinel-title = 未登録の内部 sentinel（パイプライン内部エラー）
diag-unregistered-sentinel-body =
    未登録の私用領域 sentinel が検出されました (pipeline 内部の整合性エラー)。

    これは aozora-pipeline のバグの可能性が高いです。再現手順を添えて issue で報告してください: https://github.com/P4suta/aozora/issues

diag-registry-out-of-order-title = プレースホルダーレジストリの順序破壊（パイプライン内部エラー）
diag-registry-out-of-order-body =
    プレースホルダーレジストリの順序が崩れています (pipeline 内部の整合性エラー)。

    aozora-pipeline のバグの可能性があります。再現手順を添えて issue で報告してください: https://github.com/P4suta/aozora/issues

diag-registry-position-mismatch-title = プレースホルダーレジストリの位置不一致（パイプライン内部エラー）
diag-registry-position-mismatch-body =
    プレースホルダーレジストリの位置情報が期待と異なっています (pipeline 内部の整合性エラー)。

    aozora-pipeline のバグの可能性があります。再現手順を添えて issue で報告してください: https://github.com/P4suta/aozora/issues

## doctor
##
## `aozora doctor` — 利用者向けのランタイム self-check（貢献者向けの
## `just doctor` とは別物）。節見出し・状態語・ヒントは localize するが、設定／
## ツールの識別子、enum タグ、出所ラベル（flag / env / project / global /
## default）、ツールのバージョンは機械語彙でどのロケールでもそのまま。

doctor-title = aozora doctor — ランタイム self-check
doctor-config-heading = 設定
doctor-settings-heading = 実効設定
doctor-tools-heading = 外部ツール
doctor-terminal-heading = 端末

# 設定の節。$dir は `.aozora.toml` を上方向に探索し始めた作業ディレクトリ、
# $error は不正なファイルが返す（英語の）ローダメッセージ。
doctor-project-none = なし（{$dir} から上方向に探索）
doctor-global-none = なし
doctor-parse-ok = 設定は正常に解析されました（未知のキーなし）
doctor-parse-error = 設定エラー: {$error}

# 実効設定の節。ブロッキング行: $var に $value が設定されているが、CLI ランタイム
# の clap パーサはこれを拒否する（大文字小文字を区別する値挙のミスマッチ、または
# true / false 以外の bool）。$var と $value はそのまま表示。
doctor-setting-rejected = {$var}={$value} が設定されていますが有効な値ではありません。aozora はこれを拒否します

# 外部ツール。ヒント行は見つからないツールの後に続く。
doctor-tool-missing = PATH 上に見つかりません
doctor-hint-pandoc = `aozora pandoc -t FMT` に必要。https://pandoc.org から導入
doctor-hint-lsp = `aozora lsp` に必要。aozora ツールチェインに同梱

# 端末の節。$value は設定されている場合の環境変数の生の値。
doctor-terminal-yes = 端末
doctor-terminal-no = 端末ではない
doctor-env-set = 設定あり ({$value})
doctor-env-unset = 未設定
doctor-colour-label = 実効カラー
doctor-colour-on = 有効
doctor-colour-off = 無効

# 末尾のまとめ。$count はブロッキングな問題の件数。
doctor-all-passed = すべてのチェックに合格しました。
doctor-problems = {$count} 件の問題が見つかりました。

## init
##
## `aozora init` — プロジェクトの雛形を作成。ローカライズされるのはレポートの
## 装飾のみ。生成されるファイル名・ファイルの中身・リテラルな `aozora …`
## 次ステップコマンドは言語非依存のプロジェクト成果物で、どのロケールでも同一。

init-heading = aozora init — プロジェクトの雛形を作成

# 生成した各ファイル名の前に表示する結果の語。
init-created = 作成
init-overwritten = 上書き
init-skipped = スキップ
# 既存（スキップした）ファイルの後ろの補足。`--force` はリテラルなフラグ名で
# どのロケールでも同一。
init-skipped-hint = 既に存在します。上書きするには --force

# 次のステップの案内。`aozora …` コマンドはリテラルで、ここで訳すのは末尾の
# コメントのみ。
init-next-steps = 次のステップ:
init-step-render = サンプルを HTML に変換
init-step-check = 診断を表示
init-step-doctor = 実効設定を確認

## repl
##
## `aozora repl` —対話的な read-eval-print ループ。ここはすべて人間向けの装飾:
## バナー・セクションのラベル・メタコマンドの応答・ヘルプ・その場のエラーを
## ローカライズする。ループが包む表示内容 — ノード JSON・レンダリング済み HTML・
## Pandoc AST・英語の診断レポート — は機械軸であり、ここを通さない (ADR-0033)。
## `:command` 名とその字面引数はどの言語でも同一。

# 起動時に一度表示。
repl-banner = aozora repl — 記法を入力すると nodes / HTML / 診断をすぐに表示します。コマンド一覧は :help、終了は :quit。

# `:help` — メタコマンド一覧。末尾の説明のみローカライズする。
repl-help =
    コマンド:
      :mode  nodes | html | pandoc | all   表示するビューを選ぶ
      :lang  en | ja | zh                  この画面表示の言語
      :encoding  auto | utf8 | sjis        :load で使うデコーダ
      :load  FILE                          ファイルの内容を解析する
      :help                                このヘルプを表示する
      :quit                                ループを抜ける (Ctrl-D も可)

    青空文庫記法の行を入力すると解析結果が表示されます。

# 各ビューと診断ブロックの前に付くセクションラベル。
repl-label-nodes = ノード:
repl-label-html = HTML:
repl-label-pandoc = Pandoc:
repl-label-diag = 診断:
# 解析がクリーンなときに診断ブロックへ表示するプレースホルダ。
repl-diag-none = (診断なし)

# `:mode` / `:lang` / `:encoding` 切り替え後の応答。値 ($mode / $lang /
# $encoding) は字面のタグでどの言語でも同一。
repl-mode-set = モード → {$mode}
repl-lang-set = 言語 → {$lang}
repl-encoding-set = エンコーディング → {$encoding}

# `:load` — 読み込んだファイルの評価前に表示するヘッダと、その場の
# (致命的でない) 読み込み/デコードエラー。$path はファイル、$error は英語の
# エンジンメッセージ。
repl-loaded = 読み込み {$path}
repl-load-error = {$path} を読み込めません: {$error}

# 未知の `:command` ($cmd はコロンなし) と、引数が不足/不正なときの使い方
# ($expected は受理される値の一覧)。
repl-unknown-meta = 不明なコマンド `:{$cmd}` — 一覧は :help
repl-usage = 使い方: :{$cmd} {$expected}

## TUI ライブエディタ
##
## フルスクリーン `aozora tui` のクローム: 3 ペインのタイトル、未保存マーカー、
## 診断なしのプレースホルダ、フッタのキーバインド凡例（訳すのは動詞のみ。
## ^S / ^L / ^P / ^Q のグリフや html / nodes / en といったリテラルは全ロケール
## 共通）、保存・エラーのステータス行。ペインの*中身*（HTML・ノード JSON・
## Pandoc AST・英語の診断レポート）は機械軸であり、ここは経由しない（ADR-0033）。

# ペインタイトル（ファイルパス・ビュー種別・診断件数はコード側で付加）。
tui-title-source = ソース
tui-title-preview = プレビュー
tui-title-diagnostics = 診断
# ソースタイトルに付く未保存マーカー。
tui-modified = 未保存
# 解析がクリーンなときの診断ペインのプレースホルダ。
tui-diag-none = （診断なし）

# フッタのキーバインド凡例 — 各 Ctrl グリフの後ろに置く動詞。
tui-key-save = 保存
tui-key-lang = 言語
tui-key-preview = プレビュー
tui-key-quit = 終了

# Ctrl-S 後のフッタステータス。$path は保存先、$error は英語の OS メッセージ。
# no-file 行はパスなしで開いたバッファのときに出る。
tui-saved = 保存しました {$path}
tui-save-error = 保存できません {$path}: {$error}
tui-no-file = 保存先がありません — パスを指定して開き直してください: aozora tui FILE
# stdout / stdin が端末でない（パイプ）ときの拒否。TUI には端末が必要。
tui-no-tty = aozora tui には対話端末が必要です（aozora repl か --watch を試してください）

## LSP エディタ表層
##
## aozora-lsp が出す人間向けの表層: 外字ホバー／インレイのツールチップ、
## コードアクションのタイトル、補完の detail／documentation。プロトコルの
## データ軸（カスタムメソッドの payload、診断の range／コード、semantic
## token、フォーマット編集）は決してここを通さない。文中の記法グリフや
## Tab ストップのテンプレートは青空記法そのもので全 locale 共通、周囲の
## プロースだけを訳す。

# 外字 (gaiji) 参照 — ホバー見出しと Markdown 本文のラベル。
lsp-hover-gaiji-header = **外字 (gaiji)**
lsp-hover-resolved-label = 解決
lsp-hover-composed-seq-label = 合成シーケンス
lsp-hover-unresolved = (辞書にマッチせず — 記述で代替表示)
lsp-hover-description-label = 記述
# 外字インレイヒントのツールチップ見出し（上の解決ラベルを再利用）。
lsp-inlay-gaiji-header = **外字**

# コードアクション（クイックフィックス／リファクタ）のタイトル。エディタの
# 電球メニューに出る。`SEL` は選択範囲が入る位置を示す。
lsp-action-ruby = ルビをふる ｜SEL《》
lsp-action-ruby-double = 二重ルビをふる ｜SEL《《》》
lsp-action-wrap-quote = 「」 で囲む
lsp-action-wrap-accent = 〔〕 で囲む (アクセント分解)
lsp-action-wrap-annotation = ［＃...］ 注記にする
lsp-action-bouten = 傍点を付ける ［＃「SEL」に傍点］
# $close は欠けている閉じグリフ、$open はペアの開きグリフ。
lsp-action-close-bracket = `{$close}` を補って閉じる ({$open} ペア)
# $close は対応する開きのない余分な閉じグリフ。
lsp-action-delete-unmatched = 対応のない `{$close}` を削除する
# $directive は near-miss を書き換える正準 ［＃…］ 全体。
lsp-action-rewrite = `{$directive}` に書き換える
# $codepoint は私用領域スカラーの `04X` 16 進（`U+` 接頭なし）。
lsp-action-delete-pua = 私用領域文字 U+{$codepoint} を削除する

# 補完の detail／documentation の断片。
lsp-completion-half-to-full-hint = (半角→全角)
lsp-completion-takes-param = (パラメータあり)

# 半角→全角「emmet」補完の detail。ターゲットとそれを生む半角トリガを示す。
lsp-emmet-ruby-open = ルビ読み (半角『<』→全角ペア『《》』)
lsp-emmet-ruby-close = ルビ読み閉じ (半角『>』→全角『》』)
lsp-emmet-bracket-open = 全角左ブラケット (半角『[』→全角『［』)
lsp-emmet-bracket-close = 全角右ブラケット (半角『]』→全角『］』)
lsp-emmet-ruby-base = ルビベース印 (半角『|』→全角『｜』)
lsp-emmet-gaiji-marker = 外字マーカー (半角『*』→全角『※』)
# $prefix は入力した半角文字、$glyph は全角ターゲット。
lsp-emmet-doc = 半角 `{$prefix}` → `{$glyph}`

# 構造化スニペット補完の detail／documentation。`${…}` と `<…>` は
# スニペット本文が埋める Tab ストップのスロットを表す。
lsp-snippet-empty-wrap-detail = 注記スラグの空ひな型 (中身を編集)
lsp-snippet-empty-wrap-doc = `#` を `［＃<カーソル>］` に変換。Enter で確定。
lsp-snippet-ruby-detail = ルビ ｜ベース《読み》 (Tab で読みへ移動)
lsp-snippet-ruby-doc = `｜` の後に `<base>《<reading>》` を挿入。`<base>` から開始、Tab で `<reading>` へ。
lsp-snippet-reading-detail = ルビ読み (閉じ括弧自動補完)
lsp-snippet-reading-doc = `《` の後に `<reading>》` を挿入。`<reading>` を編集。
lsp-snippet-gaiji-detail = 外字注記 (description, mencode)
lsp-snippet-gaiji-doc = `※` の後に `［＃「<desc>」、<men>］` を挿入。`<desc>` から開始、Tab で `<men>` へ。

# タイトル未入力の見出しに対するアウトラインのプレースホルダ。
lsp-outline-untitled = (無題)
