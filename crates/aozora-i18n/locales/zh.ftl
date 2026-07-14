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

## fmt 批处理 UX

# 遍历目录查找源文件时显示的旋转指示器（文件数尚未确定）。
fmt-progress-discovering = 正在查找源文件…

# 目录 fmt 运行结束后的批处理摘要。$formatted 为已格式化（在 --check / --list
# 下为将会格式化）的文件数，$unchanged 为已是规范形式的文件数，$errors 为无法
# 读取或格式化的文件数。
fmt-summary = 已格式化 {$formatted} 个，未更改 {$unchanged} 个，错误 {$errors} 个

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

# 当 `aozora explain <TARGET>` 指向解析器无法识别的对象时输出（stderr，非零
# 退出）。$target 是未能识别的参数。
explain-unknown = 未知的 explain 对象 `{$target}`
# 当存在与未知对象相近的候选（对节点标签、概念、诊断代码做编辑距离匹配）时，
# 追加到上一行。$suggestion 是最接近的已知对象，是字面标识符，不做本地化。
explain-did-you-mean = 是否想找 `{$suggestion}`?
# 指明有效集合所在的结尾。`aozora spec kinds` 与示例代码是原样的 shell 文本，
# 在每种语言中都相同。
explain-unknown-hint =
    应为 NodeKind 标签或记法概念（运行 `aozora spec kinds`），或诸如
    `aozora::lex::unclosed_bracket` 的诊断代码

## 记法概念
##
## `aozora explain <concept>` 的文字。面向读者常输入、但与 NodeKind 手册页并非
## 一一对应的记法族 —— 缩写（`tcy`）与日文名称（`傍点`、`ルビ` …）。每个族有一组
## `concept-<slug>-title`（标题）与 `concept-<slug>-body`（简短文字）。其中的青空
## 记法字形是字面语法，在每种语言中都相同。

concept-ruby-title = 注音 (ルビ / ruby) — 读音注记
concept-ruby-body =
    在主文旁标注的小号读音: 青空《あおぞら》。当范围有歧义时，用开头的 `｜`
    明确指定注音的主文: ｜青空《あおぞら》。

    运行 `aozora explain ruby` 查看完整手册页。

concept-gaiji-title = 外字 (gaiji) — 非 Unicode 字符引用
concept-gaiji-body =
    无法用普通 Unicode 书写的字符，写作 ※［＃…］ 引用 —— 通常是 JIS X 0213 的
    面区点（`第3水準1-15-23`）或 `U+XXXX` 代码。

    运行 `aozora explain gaiji` 查看完整手册页；另见诊断 `unresolved_gaiji`。

concept-kaeriten-title = 返り点 (kaeriten) — 汉文返读符号
concept-kaeriten-body =
    用于按日语语序阅读汉文的返读符号，写作 ［＃…］ 注记 —— 有 レ 点以及
    一/二、上/下、甲/乙 各族。

    运行 `aozora explain kaeriten` 查看完整手册页；另见诊断
    `bracketed_kaeriten_no_pair`、`kaeriten_outside_kanbun`。

concept-bouten-title = 傍点 (bouten) — 强调点
concept-bouten-body =
    标在文字旁的强调点，相当于青空记法中的斜体: ［＃「ここ」に傍点］。同族的
    傍線 画线而非点。

    运行 `aozora explain bouten` 查看完整手册页；另见诊断 `bouten_target_ambiguous`。

concept-warichu-title = 割注 (warichu) — 行内夹注
concept-warichu-body =
    在正文中以半高排成两行的夹注，以 ［＃割り注］ 开始、以 ［＃割り注終わり］
    结束。

    运行 `aozora explain warichu` 查看完整手册页。

concept-tcy-title = 縦中横 (tate-chu-yoko) — 竖排中的横排
concept-tcy-body =
    在竖排文本中把一小段横排 —— 通常是两位数字 —— 直立排布的记法，写作
    ［＃「25」は縦中横］。

    NodeKind 标签为 `combineUpright`；另见诊断 `tcy_target_not_found`。

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

## doctor
##
## `aozora doctor` — 面向最终用户的运行时自检（区别于面向贡献者的
## `just doctor`）。小节标题、状态词与提示会本地化；而设置／工具的标识符、
## enum 标签、来源标签（flag / env / project / global / default）以及工具版本
## 是机器词汇，在每种语言中都保持原样。

doctor-title = aozora doctor — 运行时自检
doctor-config-heading = 配置
doctor-settings-heading = 生效设置
doctor-tools-heading = 外部工具
doctor-terminal-heading = 终端

# 配置小节。$dir 是向上查找 `.aozora.toml` 的起始工作目录；$error 是格式错误的
# 文件返回的（英文）加载器消息。
doctor-project-none = 无（从 {$dir} 向上查找）
doctor-global-none = 无
doctor-parse-ok = 配置解析正常（无未知键）
doctor-parse-error = 配置错误: {$error}

# 生效设置小节。阻塞行: $var 被设为 $value，但 CLI 运行时的 clap 解析器会拒绝它
# （区分大小写的值枚举不匹配，或并非恰好为 true / false 的布尔值）。$var 与 $value
# 保持原文。
doctor-setting-rejected = 已设置 {$var}={$value}，但不是有效值；aozora 会拒绝它

# 外部工具。提示行跟在缺失的工具之后。
doctor-tool-missing = 未在 PATH 中找到
doctor-hint-pandoc = `aozora pandoc -t FMT` 需要；从 https://pandoc.org 安装
doctor-hint-lsp = `aozora lsp` 需要；随 aozora 工具链一同提供

# 终端小节。$value 是已设置时环境变量的原始值。
doctor-terminal-yes = 终端
doctor-terminal-no = 非终端
doctor-env-set = 已设置 ({$value})
doctor-env-unset = 未设置
doctor-colour-label = 生效颜色
doctor-colour-on = 开
doctor-colour-off = 关

# 结尾摘要。$count 是阻塞性问题的数量。
doctor-all-passed = 所有检查均已通过。
doctor-problems = 发现 {$count} 个问题。

## init
##
## `aozora init` — 生成新项目脚手架。仅报告外壳会本地化：生成的文件名、文件
## 内容本身以及字面的 `aozora …` 后续命令都是与语言无关的项目产物，在任何
## 语言环境下都相同。

init-heading = aozora init — 生成项目脚手架

# 每个生成文件名之前显示的结果词。
init-created = 已创建
init-overwritten = 已覆盖
init-skipped = 已跳过
# 已跳过（已存在）文件后的括注；`--force` 是字面标志名，在任何语言环境下都相同。
init-skipped-hint = 已存在；使用 --force 覆盖

# 后续步骤提示。`aozora …` 命令为字面文本，此处仅翻译末尾注释。
init-next-steps = 后续步骤:
init-step-render = 将示例渲染为 HTML
init-step-check = 报告诊断
init-step-doctor = 检查生效配置

## repl
##
## `aozora repl` — 交互式的读取-求值-打印循环。此处全部为面向人类的外壳:
## 横幅、区块标签、元命令的回执、帮助与就地错误都会本地化。循环所包裹的视图
## 内容 — 节点 JSON、渲染后的 HTML、Pandoc AST 与英文诊断报告 — 属于机器轴，
## 绝不经过此处 (ADR-0033)。`:command` 名称及其字面参数在所有语言中保持一致。

# 启动时显示一次。
repl-banner = aozora repl — 输入记法即可立即查看 nodes / HTML / 诊断。命令列表见 :help，退出用 :quit。

# `:help` — 元命令一览，仅本地化末尾的说明。
repl-help =
    命令:
      :mode  nodes | html | pandoc | all   选择要显示的视图
      :lang  en | ja | zh                  此界面的显示语言
      :encoding  auto | utf8 | sjis        :load 使用的解码器
      :load  FILE                          解析文件内容
      :help                                显示此帮助
      :quit                                退出循环（也可用 Ctrl-D）

    输入一行青空文库记法即可看到解析结果。

# 每个视图与诊断块之前的区块标签。
repl-label-nodes = 节点:
repl-label-html = HTML:
repl-label-pandoc = Pandoc:
repl-label-diag = 诊断:
# 解析无误时诊断块中显示的占位文本。
repl-diag-none = (无诊断)

# `:mode` / `:lang` / `:encoding` 切换后的回执。值 ($mode / $lang / $encoding)
# 为字面标签，在所有语言中保持一致。
repl-mode-set = 模式 → {$mode}
repl-lang-set = 语言 → {$lang}
repl-encoding-set = 编码 → {$encoding}

# `:load` — 加载文件求值前的标题，以及就地的（非致命）读取/解码错误。
# $path 为文件，$error 为英文引擎消息。
repl-loaded = 已加载 {$path}
repl-load-error = 无法读取 {$path}: {$error}

# 未知的 `:command`（$cmd 不含冒号），以及参数缺失/无效时的用法
# （$expected 列出可接受的取值）。
repl-unknown-meta = 未知命令 `:{$cmd}` — 列表见 :help
repl-usage = 用法: :{$cmd} {$expected}

## TUI 实时编辑器
##
## 全屏 `aozora tui` 编辑器的外壳: 三个窗格的标题、未保存标记、无诊断占位符、
## 底部快捷键图例（仅翻译动作词 — ^S / ^L / ^P / ^Q 字形以及 html / nodes / en
## 等字面量在所有语言中保持一致）、保存/错误状态行。窗格*内容*（渲染后的
## HTML、节点 JSON、Pandoc AST、英文诊断报告）属于机器轴，不经过此处（ADR-0033）。

# 窗格标题（文件路径、视图标签、诊断数量由代码追加）。
tui-title-source = 源文
tui-title-preview = 预览
tui-title-diagnostics = 诊断
# 追加到源文标题上的未保存标记。
tui-modified = 未保存
# 解析干净时诊断窗格的占位符。
tui-diag-none = （无诊断）

# 底部快捷键图例 — 每个 Ctrl 字形后的动作词。
tui-key-save = 保存
tui-key-lang = 语言
tui-key-preview = 预览
tui-key-quit = 退出

# Ctrl-S 之后的底部状态。$path 为保存路径，$error 为英文操作系统消息。
# 无文件行在缓冲区打开时未带路径的情况下出现。
tui-saved = 已保存 {$path}
tui-save-error = 无法保存 {$path}: {$error}
tui-no-file = 无文件可保存 — 请带路径重新打开: aozora tui FILE
# 当 stdout / stdin 不是终端（管道）时的拒绝。TUI 需要一个 tty。
tui-no-tty = aozora tui 需要交互式终端（可尝试 aozora repl 或 --watch）

## LSP 编辑器外壳
##
## aozora-lsp 输出的面向用户的外壳文本：外字悬浮／内嵌提示的 tooltip、
## 代码操作标题，以及补全的 detail／documentation。协议数据轴（自定义
## 方法负载、诊断 range／代码、语义 token、格式化编辑）绝不经过此处。
## 文本中穿插的记法字形与 Tab 停位模板是青空记法本身，各 locale 通用，
## 只翻译周围的说明文本。

# 外字 (gaiji) 引用 — 悬浮标题与 Markdown 正文中的标签。
lsp-hover-gaiji-header = **外字 (gaiji)**
lsp-hover-resolved-label = 解析
lsp-hover-composed-seq-label = 合成序列
lsp-hover-unresolved = (未匹配字典 — 改为显示描述文本)
lsp-hover-description-label = 描述
# 外字内嵌提示的 tooltip 标题（复用上面的解析标签）。
lsp-inlay-gaiji-header = **外字**

# 代码操作（快速修复／重构）标题，显示在编辑器的灯泡菜单中。
# `SEL` 标示当前选区在穿插字形中的落点。
lsp-action-ruby = 添加注音 ｜SEL《》
lsp-action-ruby-double = 添加双重注音 ｜SEL《《》》
lsp-action-wrap-quote = 用 「」 括起
lsp-action-wrap-accent = 用 〔〕 括起（口音分解）
lsp-action-wrap-annotation = 转为 ［＃...］ 注记
lsp-action-bouten = 添加着重点 ［＃「SEL」に傍点］
# $close 为缺失的闭合字形，$open 为该对的开启字形。
lsp-action-close-bracket = 补入 `{$close}` 以闭合（{$open} 对）
# $close 为无匹配开启的多余闭合字形。
lsp-action-delete-unmatched = 删除无匹配的 `{$close}`
# $directive 为将该近似写法改写成的完整正规 ［＃…］。
lsp-action-rewrite = 改写为 `{$directive}`
# $codepoint 为私用区标量的 `04X` 十六进制（不含 `U+` 前缀）。
lsp-action-delete-pua = 删除私用区字符 U+{$codepoint}

# 补全 detail／documentation 的片段。
lsp-completion-half-to-full-hint = (半角→全角)
lsp-completion-takes-param = (带参数)

# 半角→全角「emmet」补全的 detail，标明目标及产生它的半角触发键。
lsp-emmet-ruby-open = 注音读音（半角『<』→全角对『《》』）
lsp-emmet-ruby-close = 注音读音闭合（半角『>』→全角『》』）
lsp-emmet-bracket-open = 全角左括号（半角『[』→全角『［』）
lsp-emmet-bracket-close = 全角右括号（半角『]』→全角『］』）
lsp-emmet-ruby-base = 注音基字标记（半角『|』→全角『｜』）
lsp-emmet-gaiji-marker = 外字标记（半角『*』→全角『※』）
# $prefix 为键入的半角字符，$glyph 为全角目标。
lsp-emmet-doc = 半角 `{$prefix}` → `{$glyph}`

# 结构化片段补全的 detail／documentation。`${…}` 与 `<…>` 表示
# 片段正文填充的 Tab 停位槽。
lsp-snippet-empty-wrap-detail = 注记 slug 空模板（编辑内容）
lsp-snippet-empty-wrap-doc = 将 `#` 转换为 `［＃<光标>］`。按 Enter 确认。
lsp-snippet-ruby-detail = 注音 ｜base《reading》（Tab 跳到读音）
lsp-snippet-ruby-doc = 在 `｜` 后插入 `<base>《<reading>》`。从 `<base>` 开始，Tab 跳到 `<reading>`。
lsp-snippet-reading-detail = 注音读音（自动补全闭括号）
lsp-snippet-reading-doc = 在 `《` 后插入 `<reading>》`。编辑 `<reading>`。
lsp-snippet-gaiji-detail = 外字注记（description, mencode）
lsp-snippet-gaiji-doc = 在 `※` 后插入 `［＃「<desc>」、<men>］`。从 `<desc>` 开始，Tab 跳到 `<men>`。

# 标题尚未输入的大纲占位符。
lsp-outline-untitled = (无标题)
