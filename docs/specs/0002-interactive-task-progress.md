# nix-pins：交互式任务级进度

labels: ready-for-agent

## 问题陈述

`nix-pins update` 已经把工作分成 Checker、Source Hash 与 Derived Hash 三类任务，并通过两级并发分别执行。当前终端反馈仍由 worker 直接调用 `eprintln!`：用户只能看到零散的 `hashing ...` 行，无法知道当前阶段、已完成数量和仍在运行的 Pin；并发提高后，这些行还会交错。

需要为交互式终端增加任务级进度显示，但不得改变 Pins File、部分失败、退出码、并发默认值或 Nix 求哈希机制。

## 已确认的产品决策

1. 首版只显示任务级进度，不承诺下载字节数、速度、ETA 或准确百分比。
2. 使用 `indicatif::MultiProgress` 渲染交互式终端；依赖版本使用 `0.18` 系列。
3. 每个阶段显示一个总进度条，并只显示当前活跃任务。成功任务完成后清除，失败详情在最终摘要中统一输出。
4. 每个阶段独立计数，不提供跨阶段的统一百分比。
5. stderr 是 TTY 时默认启用交互式进度，不增加开关。
6. 保持现有并发默认值：Checker 默认 8，Source/Derived Hash 默认 1。
7. 保留 `NIX_PINS_CHECKER_JOBS` 与 `NIX_PINS_HASH_JOBS`，不增加 CLI 并发参数。
8. 最终摘要只提供处理数量与失败数量，不区分 `updated`、`unchanged` 或 `hash reused`。
9. spinner、进度条宽度和阶段文案不属于稳定 CLI 输出契约；Pins File、退出码和最终失败摘要仍属于稳定行为。
10. 非 TTY 输出不在本功能中重新设计；必须保留当前纯文本行为且不得输出 ANSI 控制字符。

## 既有约束

- 遵守 ADR-0007：单个 Pin 失败时保留其上一轮完整条目，成功 Pin 正常写入，命令以非零退出码结束，stderr 最终列出失败项。
- 遵守 ADR-0008：Checker 与 Hash 使用分离的并发池，不把两类任务合并到一个全局池。
- 遵守 ADR-0009：CLI 继续只提供现有命令与过滤方式，不增加 `--progress`、`--jobs` 或类似参数。
- 不改变 Fake Hash、Intermediate FOD、drvPath 指纹、两阶段求值或 Pins File schema。
- 不引入 Tokio、async runtime、Nix `internal-json` 解析或自制 ANSI 光标控制。

## 交互式终端行为

### 阶段模型

按以下顺序显示阶段；任务数为零的阶段不创建进度条：

| 阶段 | 计数单位 | 总数来源 | 并发来源 |
| --- | --- | --- | --- |
| Checking versions | 选中的 Pin | Checker 选择结果 | `NIX_PINS_CHECKER_JOBS`，默认 8 |
| Hashing sources | 需要重算 src hash 的 Pin | 第一次 probe 与指纹比较结果 | `NIX_PINS_HASH_JOBS`，默认 1 |
| Hashing derived | 需要重算任意 Derived Hash 的 Pin | 第二次 probe 与指纹比较结果 | `NIX_PINS_HASH_JOBS`，默认 1 |
| Writing pins.json | 一次写入动作 | 固定为 1 | 串行 |

Derived 阶段继续以现有 `DerivedTask` 为一个计数单位。一个 Pin 含有多个 Derived Hash 时，不得为了让进度数字更细而拆分任务或改变失败边界；活跃任务文案可以显示当前键，例如 `foo: npmDepsHash`。

### 展示规则

- 每个阶段只有一个总进度条。
- 活跃任务行数量不得超过该阶段实际 worker 数量。
- worker 开始任务时创建或更新对应活跃行；完成后递增阶段计数并清除该行。
- 阶段完成后清除其总进度条，再进入下一阶段。
- 失败任务同样从活跃区清除；完整错误只在现有最终失败摘要中输出，避免同一错误重复两次。
- 所有交互式内容写入 stderr。不得向 stdout 写入进度信息。
- 命令完成时 stderr 不得遗留未完成的 spinner、隐藏光标状态或半行 ANSI 输出。

示意输出，不作为逐字符测试快照：

```text
Checking versions  [████████░░] 24/33
⠋ ripgrep
⠋ bat
⠋ fd
```

进入 Hash 阶段后，Checker 行应已清除：

```text
Hashing sources    [██████░░░░] 6/10
⠋ ripgrep
```

结束后只留下简短结果与既有失败摘要：

```text
Processed 33 pins; 2 failed
  foo: GitHub rate limit
  bar: source hash failed
```

上述英文文案可以在不改变语义的情况下调整，脚本不得依赖它。

## 实现边界

### 终端模块

新增一个小型终端进度模块，建议放在 `src/progress.rs`。它必须是具体实现，不增加只有一个实现的 trait、factory 或抽象层。

模块职责：

1. 使用 `std::io::IsTerminal` 判断 stderr 是否为交互式终端。
2. TTY 模式下启动唯一 renderer，并由该 renderer 独占 `indicatif::MultiProgress` 与 stderr 的动态区域。
3. worker 只能通过 `std::sync::mpsc` 发送结构化事件，不得直接操作进度条或打印动态状态。
4. 非 TTY 模式不得创建 `MultiProgress`，并继续输出当前已有的纯文本阶段与 hashing 信息。
5. 正常完成和错误返回都必须关闭 channel、结束 renderer 并等待其退出。

事件至少需要表达：

- 阶段开始及任务总数；
- Pin 任务开始；
- 当前 Derived Hash 键变化；
- Pin 任务完成；
- 阶段完成；
- 全部进度结束。

事件只描述显示状态，不得承载业务结果、Pin 数据或错误控制流。

### 执行链集成

按以下顺序集成：

1. `run_update` 创建进度上下文，并保证所有返回路径都会正常结束 renderer。
2. `update` 在 Checker、Source、Derived 与写文件边界发送阶段事件。
3. `run_checkers`、`run_sources`、`run_derived` 在现有 `parallel_map` 任务边界发送开始与完成事件。
4. `resolve_source` 与 `resolve_derived` 中现有的 `eprintln!("hashing ...")` 改为进度事件；非 TTY 模式仍渲染为等价纯文本。
5. `parallel_map` 的线程数、队列方式、结果收集方式与 `BTreeMap` 排序保持不变。
6. Pins File 仍只在现有业务流程决定写入时保存；进度 renderer 不得触碰文件系统。

为了显示最终数量，允许把 `update` 的内部返回值从失败 `BTreeMap` 扩展为只包含 `processed` 数量与原失败表的结果结构。不得顺带引入版本变化、hash 复用或耗时统计模型。

## 错误与中断

- 进度显示失败不得改变 Pin 更新结果；能继续时应降级为纯文本或无动态显示。
- worker panic 的现有传播/处理语义不得因为 renderer 改变。
- Nix、HTTP、Git 或用户 checker 的完整错误内容必须保持原样进入最终失败摘要。
- 不解析或丢弃 `nix build` stderr；Fake Hash 的 `got:` 提取逻辑保持原样。
- 不在本功能中增加自定义 Ctrl-C/signal 处理依赖。

## 测试流程

实现者按以下顺序验证：

1. 为进度模块使用 `indicatif::InMemoryTerm` 添加一个最小单元测试：阶段开始后能看到阶段与活跃 Pin，任务/阶段结束后动态行被清除。只断言必要子串和清理结果，不保存完整 ANSI 快照。
2. 增加或调整 CLI seam 测试，确认 `Command::output()` 的非 TTY stderr 不包含 ANSI escape，且现有失败摘要仍出现。
3. 运行现有部分失败测试，确认成功 Pin 写入、失败 Pin 保留旧条目、退出码非零。
4. 运行现有 `NIX_PINS_HASH_JOBS=2` 并发测试，确认进度集成没有串行化 worker。
5. 运行完整测试与格式检查：

```bash
cargo fmt --check
cargo test
```

若当前 macOS shell 的 Nix Clang 抢占系统 linker，验证时显式使用 `/usr/bin/cc`；不得把该机器特定路径写入项目配置。

## 验收标准

- 在交互式终端运行 `nix-pins update` 时，每个非空阶段显示独立进度和当前活跃 Pin。
- 同时显示的活跃 Pin 不超过当前阶段 worker 数量。
- 阶段切换和命令结束后不遗留动态行。
- 非 TTY 执行不包含 ANSI 控制字符，并保留现有纯文本诊断能力。
- `NIX_PINS_CHECKER_JOBS` 与 `NIX_PINS_HASH_JOBS` 的含义和默认值不变。
- 部分失败、Pins File 写入、错误正文、失败排序与退出码行为不变。
- 未增加 Tokio、Nix 日志解析、下载字节进度、新 CLI 参数或详细更新统计。
- `cargo fmt --check` 与 `cargo test` 通过。

## 非目标

- 下载字节数、速度、ETA 或全程统一百分比；
- 解析 `nix build --log-format internal-json`；
- 默认并行多个 Nix Hash 任务；
- 重新设计 CI、管道或日志采集场景；
- `--progress`、`--no-progress`、`--jobs` 等 CLI 参数；
- JSON、日志或机器可读进度协议；
- 更新报告、自动 PR 摘要或 `updated/unchanged/hash reused` 分类；
- 修改 Pins File schema、Reader 或用户 Nix 配置接口。
