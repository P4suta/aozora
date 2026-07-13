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

# `aozora explain <code>` 出力内の節ラベル。再現例／修正例そのものは
# aozora-spec 所有の言語中立な青空記法だが、localize された title／body の
# プロースは以下で診断コードをキーに定義する。
explain-repro-label = 再現例:
explain-fixed-label = 修正後:
explain-see-label = 参照:

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
