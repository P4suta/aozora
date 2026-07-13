### 青空文库 CLI 外壳文本 — 简体中文。
###
### 仅本地化 CLI 自身面向用户的外壳文本：stdin 守卫、`--watch` 横幅，以及
### `explain` 的脚注／小节标签。机器输出轴（json / short / 代码 / exit /
### schema / timing-json）绝不经过此处。
### 参见 docs/adr/0033-cli-output-language-policy.md。

## 输入守卫

# 当文档类子命令要在裸交互终端上读取 stdin 时显示（否则会一直阻塞等待键入）。
# $cmd 是示例中出现的子命令名（例如 "check" 或 "inspect nodes"）。
stdin-empty =
    error: 标准输入为空（正在从终端读取）
      提示: 从文件读取 →  aozora {$cmd} <FILE>
            用管道传入 →  cat f.txt | aozora {$cmd}
      全部命令:  aozora --help

## 监视模式

# `--watch` 每次重新运行之间打印到终端的横幅。$path 是被监视的文件。
watch-banner = ── 正在监视 {$path}（Ctrl-C 停止）──

## explain

# `aozora check` 的人类可读诊断之后的脚注，引导读者使用
# `aozora explain <code>`。其下逐条列出的命令行是原样的 shell 命令，不做本地化。
explain-hint-header = 提示: 运行 `aozora explain <code>` 查看详情，例如:
# 当存在的不同代码多于脚注所列时的结尾；$count 是剩余数量。
explain-hint-more = … 还有 {$count} 项

# `aozora explain <code>` 输出中的小节标签。复现／修正示例本身是 aozora-spec
# 拥有的、与语言无关的青空记法；而本地化的标题／正文文字见下方，以诊断代码为键。
explain-repro-label = 复现示例:
explain-fixed-label = 修正后:
explain-see-label = 参见:

## 诊断文字
##
## 每个诊断代码有一组 `diag-<slug>-title`（标题）与 `diag-<slug>-body`（详情）；
## <slug> 为代码末尾 `::` 段中把 `_` 换成 `-` 后的形式（例如
## `aozora::lex::unclosed_bracket` → `unclosed-bracket`）。body 中的 `{$…}`
## 占位符由使用方（CLI 的 explain、LSP 悬浮提示）从实际诊断填入。这些文字已从
## aozora-spec 剥离，以使目录 crate 保持为纯机器契约；代码、严重级别、
## `#[error]` Display 以及 JSON 从不本地化。

diag-source-contains-pua-title = 源文本中混入了私用区字符
diag-source-contains-pua-body =
    源文本中混入了私用区字符 `U+{$codepoint}`。

    该字符 (`{$char}`) 是保留码位，通常不会出现在青空文库文本中，会与 aozora-lex 的内部标记 (U+E001..U+E004) 冲突。
    它通常源自编辑器的不可见字符设置，或从别处粘贴时带入的隐藏字符。

    修正方法: 删除该字符。

diag-unclosed-bracket-title = 未闭合的开括号
diag-unclosed-bracket-body =
    存在未闭合的 `{$open}`。

    请在某处补上对应的 `{$close}`——青空记法通常要求在同一行内闭合。

    示例: {$example}

diag-unmatched-close-title = 没有对应开括号的闭括号
diag-unmatched-close-body =
    存在没有对应 `{$open}` 的 `{$close}`。

    可能的原因:
    1. 多打了一个 `{$close}` → 删除它
    2. 本应在前面的 `{$open}` 缺失 → 在正确位置补上
    3. 中间还有另一个 `{$close}`，使配对错位一级 → 检查该处的配对

diag-accent-decomposition-applied-title = 已应用重音分解（提示）
diag-accent-decomposition-applied-body =
    `〔…〕` 重音记法在清洗阶段被分解为合成后的 Unicode 字符（例如 〔e'〕→é）。

    这是预期行为（ADR-0003），仅作提示性说明。

    修正方法: 无需处理。序列化时会还原为原始的 〔…〕 形式，该转换是无损的。

diag-unresolved-gaiji-title = 外字引用无法解析
diag-unresolved-gaiji-body =
    外字引用（※［＃…］）既无法解析为 Unicode 字符，也无法解析为 JIS X 0213 的面区点。

    因此渲染时不会显示预期字形，而是原样显示说明文本。

    修正方法: 为该引用提供可解析的指定——例如 `第3水準1-15-23` 这样的面区点，或 `U+XXXX` 形式的 Unicode 引用。

diag-mismatched-container-close-title = 以不同种别关闭的容器
diag-mismatched-container-close-body =
    容器以 `{$open_kind}` 开启，却由 `{$close_kind}` 的关闭指示闭合。

    开启与关闭的族类不一致，导致范围无法正确确定。

    修正方法: 用开启时的族类来闭合——`ここから字下げ` 配 `ここで字下げ終わり`，`ここから地付き` 配 `ここで地付き終わり`，依此类推。

diag-empty-ruby-reading-title = 注音读音为空
diag-empty-ruby-reading-body =
    带底文的注音（｜底文《…》）有底文却没有读音。

    由于前面存在 `｜`，这属于真正的输入笔误，而非普通的 《》，注音会退化为纯文本。

    修正方法: 补上读音（｜青空《あおぞら》）；若不想要注音，则整体移除 ｜…《》 标记，让底文作为正文保留。

diag-nested-ruby-title = 读音内嵌套的注音
diag-nested-ruby-body =
    在注音的读音内部又开启了另一个注音（《…》）。

    注音不能嵌套，因此内层的 《…》 是问题所在；外层注音会尽量被解释。

    修正方法: 在内层的 《 之前闭合外层读音，或移除内层的 《…》。

diag-unrecognised-container-directive-title = 无法识别的容器指示
diag-unrecognised-container-directive-body =
    `［＃ここから…］` 看似容器起始，但并未指明已知的容器名（字下げ／地付き／地から N 字上げ 等）。

    输出会被保留，但不会作为容器处理，而是留作普通注记。

    修正方法: 改成已知的容器名（例如 ［＃ここから2字下げ］）。

diag-tcy-target-not-found-title = 縦中横 的对象未在前方找到
diag-tcy-target-not-found-body =
    縦中横 的前向引用（［＃「X」は縦中横］）所指的对象 X 在此前的正文中没有出现。

    由于没有可修饰的文字，该指示会退化为 Unknown 注记。

    修正方法: 对象必须出现在注记之前的同一行。请检查拼写，或将 ［＃「X」は縦中横］ 放在要修饰的文字之后。

diag-bouten-target-ambiguous-title = 傍点 对象含混不清
diag-bouten-target-ambiguous-body =
    傍点 前向引用（［＃「X」に傍点］）的对象 X 在此前出现了不止一次。

    究竟给哪一次出现加着重点并不唯一，可能会修饰到非预期的位置（解析器会按回溯规则选定其一）。

    修正方法: 改写措辞使对象唯一（例如限定为 「白い花」）。

diag-forward-referent-not-stylable-title = 前向引用对象无法就地修饰
diag-forward-referent-not-stylable-body =
    前向引用的对象 X 虽在此前存在，却无法就地修饰——它可能是注音的底文、位于前一行、处于其他结构内部，或是多个候选之一。

    注记会被保留、正文可往返还原，但修饰不会应用到先前的那次出现。

    修正方法: 将 ［＃…］ 移到对象以普通形式出现的位置旁边。

diag-break-in-single-line-container-title = 单行容器内的分页／分段
diag-break-in-single-line-container-body =
    分页／分段出现在与单行容器（`{$container}`）相同的行上。

    单行容器只对该行的剩余部分生效，因此行内的换行类指示会使容器效果失效。

    修正方法: 将分页移出该行，或使用可跨换行生效的块形式 ［＃ここから…］ … ［＃ここで…終わり］。

diag-bracketed-kaeriten-no-pair-title = 没有对应基点的方括号返点
diag-bracketed-kaeriten-no-pair-body =
    方括号返点（［＃二］／［＃下］／［＃乙］ 等）在整篇文档中都没有对应族类的基点（［＃一］／［＃上］／［＃甲］）。

    由于没有可返回的目标，它无法成立为返点。

    修正方法: 在文档中某处放置族类基点——［＃二］/［＃三］ 需要 ［＃一］，［＃下］/［＃中］ 需要 ［＃上］，［＃乙］… 需要 ［＃甲］。

diag-kaeriten-outside-kanbun-title = 出现在汉文语境外的返点
diag-kaeriten-outside-kanbun-body =
    返点（［＃二］／［＃レ］ 等）出现在非汉文的语境中——它是文档中唯一的返点，且周围是普通的假名文。

    据判断，它更可能是混入的注记，而非真正的返点。

    修正方法: 若确为真正的返点，请在汉文语境中使用；否则删除该 ［＃…］ 注记。

diag-mismatched-bouten-container-title = 傍点与傍线开闭不一致的区间
diag-mismatched-bouten-container-body =
    傍点／傍线 区间以 `{$open_family}` 开启，却由 `{$close_family}` 的关闭标记闭合。

    点与线的渲染不同，因此该区间的强调含混不清（解析器会按开启方的族类恢复）。

    修正方法: 用开启时的族类来闭合——傍点 用 ［＃傍点終わり］，傍线 用 ［＃傍線終わり］。

diag-non-canonical-directive-title = 拼写不规范的 ［＃…］ 注记
diag-non-canonical-directive-body =
    拼写不规范的 ［＃…］ 注记。规范形式为 `［＃{$canonical}］`。

    其内容被判定为把已登记的记法写成了非规范拼写（送假名偏差、同义词或拼写变体），因此作为 Unknown 注记保留；解析器不会改写其内容。

    修正方法: 请改写为 `［＃{$canonical}］`。可用 `aozora fmt --fix` 自动修正。

diag-residual-annotation-marker-title = 未归类的 ［＃…］ 注记（流水线内部）
diag-residual-annotation-marker-body =
    未归类的 ［＃...］ 注记（流水线内部）。

    它未能匹配注记词典 (gaiji_chuki) 中的关键词，或可能是笔误。

    检查步骤:
    1. 确认 ［＃ 的内容是否与 `改ページ` / `中央揃え` 等已登记关键词一致
    2. 检查是否漏写了 `第3水準1-...` 之类的 JIS X 0213 面区点代码
    3. 若仍不明确，可暂用仅说明形式 (※［＃「説明」］) 通过

diag-unregistered-sentinel-title = 未登记的内部 sentinel（流水线内部错误）
diag-unregistered-sentinel-body =
    检测到未登记的私用区 sentinel（流水线内部一致性错误）。

    这很可能是 aozora-pipeline 的缺陷。请附上复现步骤提交 issue: https://github.com/P4suta/aozora/issues

diag-registry-out-of-order-title = 占位符注册表顺序破坏（流水线内部错误）
diag-registry-out-of-order-body =
    占位符注册表的顺序已被破坏（流水线内部一致性错误）。

    这可能是 aozora-pipeline 的缺陷。请附上复现步骤提交 issue: https://github.com/P4suta/aozora/issues

diag-registry-position-mismatch-title = 占位符注册表位置不一致（流水线内部错误）
diag-registry-position-mismatch-body =
    占位符注册表条目的位置信息与预期不符（流水线内部一致性错误）。

    这可能是 aozora-pipeline 的缺陷。请附上复现步骤提交 issue: https://github.com/P4suta/aozora/issues
