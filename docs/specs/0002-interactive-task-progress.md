# nix-pins：Pin 与 Package 树形进度

labels: ready-for-agent

## 问题陈述

`nix-pins update` 已经把工作分成 Checker、Source Hash 与 Derived Hash 三类任务，并通过两级并发执行。当前终端反馈仍以全局阶段和零散 `hashing ...` 行为主，用户难以追踪单个 Pin 从 Current Version 到 Target Version 的处理状态，也看不到 Nix 下载、复制 store path 或构建 phase 的实时细节。

需要把交互式反馈改为以 Pin 为一级单元的进度视图，并在存在具名或多个 Package 时显示 Package 子树。该功能不得改变 Pins File、部分失败、退出码、并发默认值、Fake Hash 循环或 Nix 求哈希判据。

## 已确认的产品决策

1. TTY 进度以 Pin Progress 为一级展示单元。每行固定显示状态符号、声明中的 Pin 名、Current Version、Target Version 与 Pin Step，不显示 `PIN CURRENT TARGET STEP` 表头。
2. 尚未锁定的 Current Version 显示为 `—`；Checker 尚未得到 Target Version 时显示为 `…`。Target Version 是候选输入，不表示已经写入 Pins File。
3. Pin Step 由 nix-pins 定义为 Checking、Version Selected、Hashing Source、Source Ready、Source Reused、Hashing Derived、Done 与 Failed。Hashing Derived 显示为 `Hashing <key>`。
4. Nix activity 只补充当前 Pin Step 的详情，不能取代 Pin Step 或决定业务结果。
5. TTY 动态区域只显示当前活跃 Pin。等待中的 Pin 不占用终端行；Done 或 Failed 后离开动态区域并转为普通静态行。
6. Failed 静态行只显示失败发生时的 Pin Step；Package 失败时保留 `Package/Derived Hash` 定位。完整错误仍由命令结束后的失败摘要统一输出。
7. Done 只表示该 Pin 的全部计算结果已经准备好，不表示已经持久化。写入结果由独立的 Writing Pins File Global Operation 表达。
8. 批量 Probe 求值、写入 Pins File 等无法归属于单个 Pin 的工作使用唯一一条临时 Global Operation 状态行，不复制成每个 Pin 的 Step。
9. 所有 `nix build` 统一使用 `--log-format internal-json`。TTY 将事件投影到动态进度；非 TTY 使用同一解析器恢复低频纯文本里程碑和诊断。
10. 使用现有 `serde_json::Value` 实现最小逐行解析器，不引入 `nixcloud/logone` 或另一套终端渲染状态。
11. 同一 Pin 的多个 file-transfer activity 聚合显示。只有全部活跃传输总量均已知且大于零时才显示汇总 `done/total`；否则只显示已下载字节和活跃传输数，不计算百分比或 ETA。
12. 进度行不显示原始 URL、Fetcher target、drvPath 或 Package 构建产物名。URL 可能过长，也可能包含签名参数或凭证。
13. 同一 Pin 存在多个 Nix activity 时，详情优先级固定为 Downloading、Copying Store Path、stdenv Phase、Building、Querying Cache。高优先级结束后回落到仍活跃的下一项。
14. Derived Hash 工作保留 Package 归属。唯一名为 `default` 的 Package 内联到父级 Pin；存在多个 Package，或唯一 Package 不是 `default` 时展开 Package 子树，父级显示 Hashing Packages。
15. Package 子树只在父级 Pin 活跃时显示。父级 Done 或 Failed 后折叠为静态 Pin 行；成功时可以显示 Package 数量。
16. Package 树不改变并发模型。同一 Source 内的 Derived Hash 继续顺序计算；`NIX_PINS_DOWNLOAD_JOBS` 控制 Source Hash 并发，`NIX_PINS_HASH_JOBS` 控制 Derived Hash 并发。
17. stderr 为 TTY 时默认启用交互式进度，不增加开关。
18. 使用 `indicatif::MultiProgress` 渲染 TTY，依赖保持 `0.18` 系列。
19. Checker 与 Source Hash 默认并发 8，Derived Hash 默认并发 1；保留 `NIX_PINS_CHECKER_JOBS`、`NIX_PINS_DOWNLOAD_JOBS` 与 `NIX_PINS_HASH_JOBS`，不增加 CLI 并发参数。
20. spinner、树形字符、列宽和具体英文文案不属于稳定 CLI 输出契约。Pins File、退出码和最终失败摘要仍属于稳定行为。

## 既有约束

- 遵守 ADR-0007：单个 Pin 失败时保留其上一轮完整条目，成功 Pin 正常写入，命令以非零退出码结束，最终摘要列出失败项。
- 遵守 ADR-0008：Checker 与 Hash 使用分离的并发池，不合并为一个全局池。
- 遵守 ADR-0009：不增加 `--progress`、`--no-progress`、`--jobs` 或类似 CLI 参数。
- 不改变 Fake Hash、Intermediate FOD、drvPath 指纹、两阶段求值、Pins File schema 或 Reader。
- 不引入 Tokio、async runtime、自制 ANSI 光标控制或 Package 级 worker。
- 当前 Checker、Source Probe、Source Hash、Derived Probe、Derived Hash 的全局屏障保持不变；Pin Progress 只是执行状态的投影，不把 Update 改造成逐 Pin 流水线。

## TTY 展示模型

### Pin 行

Pin 行不显示表头，固定顺序为：

```text
状态符号  Pin 名  Current Version  Target Version  Pin Step · Nix 详情
```

示例：

```text
⠋ curlie  v1.8.1  v1.8.2  Hashing source · Downloading 6.2/18.4 MiB
✓ ripgrep  14.1.0  14.1.1  Done
✗ broken  v3.0.0  v3.1.0  Failed · Hashing source
```

Pin 名必须来自声明键。Fetcher 的 owner/repo、URL、drvPath 与 Package 名都不能替代 Pin 名。

### Global Operation

同一时刻最多显示一条 Global Operation：

```text
⠋ Loading configuration
⠋ Resolving sources · 12 pins
⠋ Resolving derived hashes · 10 pins
⠋ Writing pins.json
```

Global Operation 完成后清除动态行；写入完成时可以留下成功或失败静态行。Pin 的 Done 不等价于该写入已成功。

### Package 子树

唯一 `default` Package 保持内联：

```text
⠋ demo  v1.0.0  v1.1.0  Hashing vendorHash · buildPhase
```

多个 Package 或唯一具名 Package 展开树：

```text
⠋ cpa-manager-plus  v1.12.1  v1.13.0  Hashing packages
  ├─ ✓ manager-server  vendorHash ready
  └─ ⠋ web             Hashing npmDepsHash · buildPhase
```

Package 树展示该 Pin 的全部 Package 状态，但同一 Pin 内仍只有一个 Derived Hash 正在执行。父级结束后折叠：

```text
✓ cpa-manager-plus  v1.12.1  v1.13.0  Done · 2 packages
✗ cpa-manager-plus  v1.12.1  v1.13.0  Failed · web/npmDepsHash
```

### Nix 详情

解析器维护一个 `nix build` 子进程内的活跃 activity 集合。由于当前每个 Hash 任务各自启动一个 `nix build`，子进程边界就是 Pin 或 Package 子任务最可靠的归属键。

详情选择顺序：

1. Downloading：聚合全部活跃 file-transfer，或解析 fetchurl 的 curl 字节进度及 fetchgit/Git LFS 的对象进度。
2. Copying Store Path：存在活跃 copy-path 或 substitute 复制。
3. stdenv Phase：显示最近仍适用的 `resSetPhase`，如 `buildPhase`。
4. Building：存在活跃 build activity，但没有更高优先级详情。
5. Querying Cache：存在查询或等待 substitute 的 activity。
6. 无详情：只显示 Pin Step。

总下载量未知时示例：

```text
Hashing source · 6.2 MiB downloaded · 2 transfers
```

不得通过缺失或为零的总量计算百分比，也不承诺速度或 ETA。

## `internal-json` 解析边界

解析器按 stderr 中逐行的 `@nix <JSON>` 记录工作，并满足以下要求：

- 使用宽松的 `serde_json::Value` 读取 `action`、`id`、`parent`、`type` 与 `fields`。
- 忽略未知 action、activity type、result type 与多余字段；未知进度只降低展示精度。
- malformed JSON、非 `@nix` stderr 行和无法结构化的诊断必须保留为普通诊断文本，不能使进度解析器改变构建结果。
- `msg`、`resBuildLogLine` 与其他可读诊断按原有顺序保存，用于非 TTY 输出、最终错误正文与既有 `got:` Hash 提取。
- file-transfer 的 `done/expected` 只服务于显示；Hash 成功仍以构建日志中解析到有效 SRI `got:` 行为判据。
- stdout 与 stderr 必须在子进程运行期间持续排空，避免因管道缓冲导致死锁；命令退出后再合并业务所需的诊断。
- 不将原始 `internal-json` 事件暴露为稳定或机器可读的公共进度协议。

解析层应集中在 Nix 交互模块附近，并返回项目自己的少量事件或状态更新；不得让 `serde_json::Value`、Nix 数字 type 或 `@nix` 细节扩散到 Update 编排和终端渲染代码。

## Probe 的 Package 归属

当前 evaluator 知道 Package 名，但 Rust Probe 只接收扁平的 `Derived Hash key → drvPath`。单一同类 Package 的 key 可能只是 `vendorHash`，无法从字符串可靠恢复 Package。

内部 Probe JSON 必须显式表达：

```text
Pin
└─ Package
   └─ Derived Hash key → drvPath
```

Rust 任务仍需携带既有 Pins File Derived Hash key，以保持缓存查找、fingerprint 与序列化结果不变。该调整只修改 evaluator 到 Rust 的内部契约，不修改用户配置 DSL、Pins File schema 或 Reader。

## 进度模块

`src/progress.rs` 继续作为具体终端实现，不新增只有一个实现的 trait、factory 或抽象层。

模块职责：

1. 使用 `std::io::IsTerminal` 判断 stderr 是否为 TTY。
2. TTY 模式由唯一 renderer 独占 `indicatif::MultiProgress` 和 stderr 动态区域。
3. Checker、Hash worker 与 Nix 日志读取线程只能发送结构化进度事件，不能直接操作进度条。
4. renderer 维护 Global Operation、活跃 Pin、可选 Package 子树和 Nix activity 投影。
5. Done 与 Failed 时清除动态树并输出一条静态 Pin 行。
6. 非 TTY 不创建 `MultiProgress`，不输出字节更新或 ANSI，只输出低频里程碑和诊断。
7. 正常完成、错误返回与 Drop 都必须关闭 channel、结束 renderer 并等待其退出。

进度事件只描述显示状态，不承载 Pin 业务数据、Hash 结果或错误控制流。终端 renderer 不触碰 Pins File。

## 执行链集成

1. `run_update` 创建进度上下文，并保证所有返回路径都会关闭 renderer。
2. Update 开始时注册选中的 Pin 名与 Current Version；Checker 开始时将 Pin 设为 Checking，完成时设置 Target Version 或 Failed。
3. 两次批量 Probe 使用 Global Operation，分别显示 Resolving Sources 与 Resolving Derived Hashes。
4. Source Hash worker 将 Pin 设为 Hashing Source；缓存复用时使用 Source Reused，否则由 Nix activity 更新详情。
5. Derived Hash worker 依据 Probe 的 Package 归属选择内联 Pin 行或 Package 子树，并在每个 Hash 开始时设置 `Hashing <key>`。
6. Pin 计算成功后输出 Done 静态行；Pin 失败后输出只含失败位置的 Failed 静态行。
7. Pins File 保存边界使用 Writing Pins File Global Operation，成功或失败不回写已经输出的 Pin 静态行。
8. `parallel_map` 的线程数、队列方式、结果收集方式和 `BTreeMap` 业务排序保持不变。

## 测试要求

1. 使用 `indicatif::InMemoryTerm` 覆盖活跃 Pin 行、完成后静态输出、Global Operation 和 Package 树折叠，不锁定整段 ANSI 快照。
2. 逐行解析器使用固定的 Nix `internal-json` 样本覆盖 start、result、stop、parent、未知 type、缺字段、malformed JSON 与普通 stderr。
3. 下载聚合覆盖多 transfer、总量全部已知、部分未知、activity stop 后回落和 URL 不进入渲染文本。
4. Nix 详情优先级覆盖 Downloading、Copying Store Path、Phase、Building 与 Querying Cache 的回落顺序。
5. `got:` 提取使用真实 `resBuildLogLine` 样本，继续覆盖有效 mismatch、404 无 got 行和 npm lockfile 失败。
6. Probe 测试覆盖唯一 `default` Package、唯一具名 Package、多个同类 Package 和不同 Builder Package，验证 Package 归属与既有扁平 Pins File key 同时正确。
7. CLI 集成测试覆盖 TTY 无 ANSI 泄漏、非 TTY 无字节刷屏、部分失败、写入失败与 `NIX_PINS_HASH_JOBS=2`。
8. `cargo fmt --check` 与 `cargo test` 通过。

## 验收标准

- TTY 中用户能从每条活跃 Pin 行读出 Pin 名、Current Version、Target Version 和当前 Pin Step。
- 多 Package Pin 在 Derived Hash 期间显示可靠的 Package 树，完成后折叠为 Pin 静态行。
- 下载总量已知时显示聚合字节进度；未知时显示不定进度且不伪造百分比。
- 原始 URL、drvPath、Fetcher target 和完整错误正文不进入动态进度行。
- 非 TTY 不包含 ANSI 或高频字节更新，并保留完整诊断能力。
- `internal-json` 解析失败或出现未知事件时，最坏结果只是详情降级，不改变 Hash 判定、Pins File、部分失败或退出码。
- 现有并发环境变量的含义与默认值不变，Package 树不增加并发。

## 非目标

- 准确速度、ETA 或跨 Pin 的统一百分比；
- 将 `internal-json` 暴露为公共 JSON、日志或机器可读进度协议；
- 增加 `--progress`、`--no-progress`、`--jobs` 等 CLI 参数；
- 把现有全局阶段屏障改造成逐 Pin 流水线；
- Package 级并发；
- 修改 Pins File schema、Reader 或用户 Nix 配置 API；
- 重新设计 CI、管道或日志采集格式；
- 在本次功能中处理 Ctrl-C 或 signal 转发。
