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

# `aozora explain <code>` 输出中的小节标签。周围的诊断文字（标题／正文／复现
# 示例／修正示例）由 spec 拥有，不在此处本地化。
explain-repro-label = 复现示例:
explain-fixed-label = 修正后:
explain-see-label = 参见:
