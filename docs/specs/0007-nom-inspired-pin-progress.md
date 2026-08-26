# nix-pins：nom 风格的 Pin Progress

labels: ready-for-agent

## 问题陈述

使用者执行 Update 时，当前 prodash 界面会把 Loading、Resolving、Writing 等 Global Operation 的完成消息逐条留在终端历史中，并把不同长度的 origin 右对齐。结果虽然完整，却不够紧凑，真正重要的 Pin、Source、Package 状态容易被过程消息淹没。

使用者希望获得类似 nom 的体验：运行期间只维护一块稳定、持续重绘的状态区域；完成的工作仍以树状结构表达；命令结束后留下可读的最终结果。同时，nix-pins 展示的是 Pin、Source 与 Package 的领域归属，而不是真实的 Nix derivation Dependency Graph。

## 解决方案

保留 prodash 31 line renderer 与现有 Pin Progress API，把进度展示改造成 nom 风格的单棵动态树。TTY 中以 `Pin Progress` 为标题，Global Phase 合并到标题行；Pin 完成后仍留在树中，多 Source Pin 保留 Source 终态，成功 Package 折叠，失败 Package 保留失败分支。树按终端高度使用软预算，活动与失败 Pin 优先保留。

命令结束时关闭动态 renderer，再从同一份 Progress 状态输出一次静态最终树、完整失败详情与总摘要。非 TTY、重定向和 CI 不绘制树，只输出低频线性阶段日志、Pin 结果、失败详情与总摘要。

## 用户故事

1. 作为交互式执行 Update 的使用者，我希望终端中只有一块持续更新的 Pin Progress，以便过程消息不会淹没真正的结果。
2. 作为使用者，我希望标题显示当前 Global Phase，以便知道 Update 正在加载配置、检查版本、解析源码、计算 Derived Hash 还是写入 Pins File。
3. 作为使用者，我希望同时看到多个 Pin，以便理解并发 Update 的整体进展。
4. 作为使用者，我希望每个 Pin 始终使用配置中声明的 Pin 名，以便显示名称与 Selection、Pins File 和错误报告保持一致。
5. 作为使用者，我希望目标 Version 已知后看到 `current → target`，以便立即识别版本变化。
6. 作为使用者，我希望未变化的 Pin 只显示一次 Version，以便避免重复文本和多余的 `unchanged` 标签。
7. 作为首次锁定 Pin 的使用者，我希望看到 `— → target`，以便明确这是首次写入而不是从某个未知版本升级。
8. 作为使用者，我希望等待、活动、成功与失败分别由 `⏸`、`⏵`、`✔`、`⚠` 表达，以便状态在禁用颜色时仍然清晰。
9. 作为使用者，我希望 Pin 完成后仍保留在树中，以便可以在一次 Update 结束前回顾已经完成的结果。
10. 作为多 Source Pin 的使用者，我希望完成后仍看到每个 Source 的最终状态，以便确认各个源码获取单元是否成功。
11. 作为仅有 `default` Source 的使用者，我希望该层继续折叠到 Pin，以便常见情况保持紧凑。
12. 作为包含多个 Package 的使用者，我希望活动 Package 在工作期间显示，以便定位当前 Derived Hash 工作。
13. 作为使用者，我希望成功 Package 完成后折叠，以便最终树不会被无问题的细节撑满。
14. 作为遇到 Package Failure 的使用者，我希望失败 Package 分支保留在最终树中，以便快速定位具体失败位置。
15. 作为使用者，我希望活动详情只显示在最深可见节点，以便 Pin、Source 和 Package 不重复同一句状态。
16. 作为使用者，我希望 bytes、Git objects 与 LFS objects 继续显示结构化计数和吞吐信息，以便评估当前传输进度。
17. 作为使用者，我希望多个并行传输继续在所属 Source 或 Package 节点聚合，以便界面不会暴露 URL 或创建大量传输子节点。
18. 作为小终端使用者，我希望 Pin Progress 按终端高度裁剪，以便动态区域不会占满整个终端。
19. 作为使用者，我希望活动和失败 Pin 不因高度预算被隐藏，以便重要状态始终可见。
20. 作为使用者，我希望成功 Pin 按最近完成优先参与可见性选择，以便有限空间显示最相关的已完成结果。
21. 作为多 Source Pin 的使用者，我希望其完整 Source 子树一起显示或隐藏，以便不会看到残缺的终态。
22. 作为使用者，我希望所有可见 Pin 仍按名称稳定排序，以便状态变化不会让整棵树不断跳动。
23. 作为使用者，我希望标题在发生裁剪时显示 `showing N of M pins`，以便知道还有 Pin 未显示。
24. 作为使用者，我希望全部 Pin 可见时标题只显示 `N pins`，单 Pin 时省略数量，以便标题保持简洁。
25. 作为窄终端使用者，我希望过长文本按显示列宽截断并追加省略号，以便自动换行不会破坏动态重绘。
26. 作为使用者，我希望状态、名称、Version 与 counter 比 detail 更优先保留，以便截断后仍能理解关键事实。
27. 作为使用者，我希望调整终端大小后在下一次业务状态变化时重新布局，以便无需额外的 signal handler 也能逐步适应新尺寸。
28. 作为使用者，我希望动态刷新保持流畅且不过度消耗资源，以便长时间 Update 不造成不必要的 CPU 开销。
29. 作为使用者，我希望命令结束后在终端历史中保留一次静态最终树，以便动态区域消失后仍能检查结果。
30. 作为遇到 Pin Failure 的使用者，我希望最终树只显示简短失败位置，完整错误集中显示在 `Failures:` 区块，以便结果既易扫描又可诊断。
31. 作为使用者，我希望 `Processed N pins; M failed` 始终是成功进入 Pin 处理后的最后一行，以便快速确认总体结果。
32. 作为遇到 Configuration Error 的使用者，我希望只看到失败的 Global Phase 和完整错误，不看到虚构的 Pin 或 `Processed 0 pins`，以便输出符合真实执行阶段。
33. 作为 CI 使用者，我希望非 TTY 输出稳定的低频线性日志，以便日志可读、可搜索且不包含动态控制序列。
34. 作为 CI 使用者，我希望非 TTY 不输出高频 bytes 或 objects 更新，以便构建日志不会膨胀。
35. 作为使用 `NO_COLOR` 的使用者，我希望所有状态仍可仅凭符号理解，以便颜色始终只是装饰。
36. 作为 Status 命令使用者，我希望它继续保持即时线性输出，以便简单只读命令不会被不必要地改造成 dashboard。

## 实现决策

- 遵循 ADR-0023，保留 prodash line renderer，只借鉴 nom 的动态展示语义，不实现 nom 的自定义 renderer。
- `Pin Progress` 是唯一界面术语；不得将其称为 Dependency Graph，因为树边表示 Pin、Source 与 Package 的领域归属，而不是 derivation 依赖。
- Progress 状态保存所有 Pin 的完整展示事实，并作为动态树与最终静态树的唯一来源。
- prodash Item 只是当前可见节点的投影。可见节点可以随软预算删除并重建，但不增加 backend trait、renderer 抽象或可切换后端接口。
- Global Phase 固定为 `Loading configuration`、`Checking versions`、`Resolving sources`、`Resolving derived hashes`、`Writing pins.json` 与 `done`。
- Global Phase 合并到标题行，不再为 Loading、Resolving 或 Writing 创建独立 Operation 节点，也不把这些完成状态写入 TTY scrollback。
- 标题格式为 `Pin Progress · {phase}`；多个 Pin 全部可见时追加 `N pins`，裁剪时追加 `showing N of M pins`，单 Pin 时省略数量。
- Pin 状态使用 Waiting、Active、Success、Failure 四种展示状态，分别格式化为 `⏸`、`⏵`、`✔`、`⚠`。
- 不调用会写入 prodash message 的 `done()` 或 `fail()` 表达 Pin 终态；完成和失败状态保存在 Progress 状态中，并通过节点名称投影。
- 状态语义不得依赖颜色。动态颜色继续由 prodash 控制并遵守终端能力与 `NO_COLOR`，不手写 ANSI 状态颜色。
- Pin Version 仅在目标与当前不同时显示箭头；未变化时只显示一次 Version；首次锁定显示 `— → target`。不追加 `done`、`updated` 或 `unchanged`。
- 唯一 `default` Source 继续折叠。多 Source Pin 完成后保留全部 Source 的成功或失败状态。
- Package 在活动期间显示。成功 Package 完成后折叠；失败 Package 在最终树中保留失败分支及 Derived Hash key。
- 活动详情只显示在最深可见节点。被折叠的 `default` 层将详情上浮到最近的可见父节点。
- bytes、Git objects 与 LFS objects 继续使用结构化 counter；并行传输继续按所属节点聚合，且仅当全部 transfer 的 total 已知时提供聚合 total。
- 不增加 Builds、Downloads、Uploads 或 Host 底部汇总表，也不暴露 URL、drvPath 或底层 Nix activity 子树。
- 可获取终端尺寸时，Pin Progress 的软高度预算为终端行数除以三；无法获取时使用 20 行。标题计入预算。
- 活动和失败 Pin 永不因预算隐藏，并允许树临时突破软预算。成功 Pin 按最近完成优先选择。
- 成功的多 Source Pin 只有完整子树能放入剩余预算时才显示。选出可见 Pin 后，Pin、Source 与 Package 均按名称稳定排序。
- 动态树和最终静态树共用同一裁剪规则。
- 终端尺寸在启动时读取，并在节点声明、Global Phase 切换、完成或失败等结构变化时重新读取。不启用 signal-hook，也不增加独立 resize 监听线程。
- 高频 counter 更新只原地修改当前可见 prodash Item，不重新读取终端尺寸或重建可见投影。
- 节点文本按 Unicode 显示宽度截断并追加 `…`。状态、名称、Version 与 counter 优先保留，detail 最先截断。
- 直接声明依赖图中已有的 Unicode width 依赖，不自行实现终端字符宽度算法。
- prodash line renderer 保持 10 FPS、无初始延迟、开启 throughput、保持光标可见，并继续不启用 signal-hook。
- TTY Update 结束时先关闭动态 renderer，再从 Progress 状态打印一次静态最终树。
- Pin Failure 的完整错误紧随最终树，集中放在 `Failures:` 区块；总摘要始终作为成功进入 Pin 处理后的最后一行。
- Configuration Error 或 Pins File 读取错误发生在 Pin 处理前时，TTY 只保留 `⚠ Pin Progress · {phase}` 和完整错误；不创建 Pin 节点，不输出 `Processed 0 pins`。
- 非 TTY、重定向和 CI 使用低频线性日志：每个 Global Phase 和 Pin 完成时各输出一行，不输出动态帧、高频 counter、ANSI 或树形连接符；随后输出失败详情与总摘要。
- 此功能只改变 Update；Status、Pins File schema、Nix 配置 DSL、Selection、并发任务数量、Checker、Fetcher 与 Builder 语义保持不变。
- 实现集中在现有 Progress 模块和 Update 编排中，只直接声明 Unicode width 依赖；不新增业务模块或通用 dashboard 框架。

## 测试决策

- 使用一个主要 seam：Progress。测试构造可注入 writer、TTY 状态和终端尺寸的 Progress，通过真实 Global Phase 与 Reporter API 驱动状态，不绕过生产状态转换。
- 测试断言面向使用者的逻辑输出和树结构，不断言 prodash 每一帧的 ANSI 控制序列、线程调度、精确刷新时间或颜色实现。
- 延续现有 Reporter 与 prodash Root snapshot 单元测试先例，但把断言提升到 Progress 可见投影和最终输出层。
- 验证单 Pin 的等待、活动、版本变化、未变化、首次锁定、成功和失败文本。
- 验证唯一 `default` Source 与 Package 折叠。
- 验证多 Source Pin 完成后保留 Source 层，并按名称稳定排序。
- 验证成功 Package 完成后折叠，失败 Package 及 Derived Hash key 保留。
- 验证活动详情只显示在最深可见节点，并在默认层折叠时正确上浮。
- 验证终端高度三分之一预算、20 行 fallback、标题计入预算、成功 Pin 最近完成优先、活动和失败 Pin 不被裁剪。
- 验证多 Source Pin 作为完整子树显示或隐藏，不出现部分 Source。
- 验证发生裁剪和未裁剪时的标题计数，以及单 Pin 时省略数量。
- 验证 Unicode 宽度截断优先级，确保长文本不会超过提供的终端列宽。
- 验证高频 counter 更新不改变可见 Pin 集合，并保留 current、total、unit 与 throughput 所需事实。
- 验证最终静态树、`Failures:` 区块和总摘要的输出顺序。
- 验证 Pin 处理前的全局错误不产生 Pin 节点或 `Processed 0 pins`。
- 验证非 TTY 只产生低频线性日志，不包含 ANSI、动态清行序列、树形连接符或高频 counter。
- 保留现有 CLI 测试验证退出码、部分失败语义和 Pins File 内容；不通过 CLI 测试重复覆盖 Progress 的全部展示组合。
- 不新增 PTY、真实终端、逐帧 snapshot 或时间敏感测试。

## 范围外

- 自定义终端 renderer、alternate screen 或完整复制 nom 布局。
- 真实 Nix derivation Dependency Graph。
- Builds、Downloads、Uploads 或 Host 汇总表。
- 原始 Nix 日志 scrollback 区域。
- 键盘交互、折叠控制、滚动、选择或暂停。
- signal-hook、即时 SIGWINCH resize 或光标隐藏。
- 新的 CLI 参数、环境变量或用户可配置主题。
- renderer/backend 抽象层或切换回 indicatif 的能力。
- 对 Status 或未来命令提供通用 dashboard。
- 修改 Pins File、配置 DSL、Selection、并发或部分失败语义。

## 进一步说明

- 本规范细化并取代旧 prodash 规范中“TTY 完成项通过持久 message 进入 scrollback”的展示决策；prodash 依赖、结构化 Nix counter、聚合 total、10 FPS、throughput、无 signal-hook 和不隐藏光标等决定继续有效。
- ADR-0023 是本规范的架构依据，项目领域术语表中的 Pin Progress、Source Progress 与 Package Progress 是实现和测试命名基准。
- nom 是交互语义和软高度预算的参考，不是领域模型或 renderer 实现模板。
