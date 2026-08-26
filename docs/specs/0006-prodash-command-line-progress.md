# nix-pins：使用 prodash 优化命令行进度可视化

labels: ready-for-agent

## 问题陈述

使用者在执行 Update 时需要同时理解多个 Pin、Source、Package 与 Nix 构建活动的状态。现有界面虽然表达了这些领域层级，但由 indicatif、自制事件队列和自制 Renderer 共同维护展示状态；下载与 Git 进度多数被压成字符串，无法稳定利用吞吐率、比例和 ETA。TTY 与非 TTY 还有独立渲染路径，增加了行为漂移和维护成本。

使用者希望界面具有类似 nom 的紧凑、流畅和可定位体验，同时仍以 nix-pins 的 Pin → Source → Package 为主视图，而不是改造成完整的 Nix derivation activity 图。

## 解决方案

使用 prodash 31 的 progress tree 和 line renderer 直接替换 indicatif 与自制 Renderer。保留 `Progress`、`Reporter`、`Operation` 及 Pin/Source/Package 领域 API，但由它们内部直接创建、更新和删除 prodash 节点。

TTY 在普通终端滚动区内以 10 FPS 动态刷新，不进入 alternate screen，也不接管键盘。非 TTY 使用同一个 line renderer，只输出 Pin 与 Global Operation 的持久消息。Nix file transfer、curl、Git 与 LFS 进度转换为结构化计数，由 prodash 显示比例、吞吐率和 ETA；Nix activity 继续只是 Source 或 Package 的次级详情。

## 用户故事

1. 作为执行 Update 的使用者，我希望同时看到多个 Pin 的活动状态，以便知道命令仍在推进。
2. 作为使用者，我希望 Pin 行显示 Current Version、Target Version 与当前 Pin Step，以便判断是否存在版本变化。
3. 作为使用者，我希望界面仍以 Pin 为一级单元，以便进度与原子 Update 语义一致。
4. 作为使用者，我希望多 Source Pin 展开具名 Source，以便定位当前正在处理的源码。
5. 作为使用者，我希望多 Package Source 展开具名 Package，以便定位正在计算的 Derived Hash。
6. 作为简单 Pin 的使用者，我希望唯一 `default` Source 和 Package 继续折叠，以便常见场景保持紧凑。
7. 作为使用者，我希望节点名称随步骤和详情更新，以便一行内获得当前上下文。
8. 作为使用者，我希望已知总量的下载显示 current/total、比例、吞吐率与 ETA，以便评估剩余时间。
9. 作为使用者，我希望未知总量的下载至少显示已下载字节与吞吐率，以便确认下载没有停滞。
10. 作为使用者，我希望 Git 与 LFS 显示 objects current/total 与速率，以便理解源码获取进度。
11. 作为使用者，我希望并行下载聚合为当前 Source 或 Package 的一个计数器，以便界面不会被传输任务淹没。
12. 作为使用者，我希望看到并行 transfer 数量，但不看到原始 URL，以便兼顾诊断信息与隐私。
13. 作为使用者，我希望 Copying、Building、Querying 与 stdenv phase 仍显示为文本详情，以便知道 Nix 当前阶段。
14. 作为使用者，我希望版本发生变化的成功 Pin 获得明显成功提示，以便快速识别更新。
15. 作为使用者，我希望版本未变化的成功 Pin 降低强调但仍显示 Done，以免误解为失败或未处理。
16. 作为使用者，我希望失败消息标明所属 Pin 与 Source/Package 位置，以便快速定位问题。
17. 作为使用者，我希望 Source 与 Package 完成后从活动树消失，以便只保留仍在工作的节点。
18. 作为使用者，我希望 Pin 与 Global Operation 的最终结果保留在 scrollback，以便命令结束后仍可检查。
19. 作为在终端中工作的使用者，我希望界面不进入全屏模式也不接管键盘，以便保留 shell 上下文。
20. 作为在 CI 或管道中运行命令的使用者，我希望不输出动态 ANSI 帧和颜色，以便日志稳定可读。
21. 作为脚本作者，我希望进度继续写入 stderr，以便 stdout 保留给结构化输出和管道。
22. 作为使用 `NO_COLOR` 的使用者，我希望进度遵循终端颜色约定，以便符合环境偏好。
23. 作为按 Ctrl-C 终止命令的使用者，我希望信号行为不因进度库改变，以免命令停止不可靠。
24. 作为使用者，我希望渲染器 I/O 故障不改变 Pin 结果、退出码或 Pins File，以便观察层不会影响业务结果。
25. 作为维护者，我希望只有一条 TTY/非 TTY 渲染路径，以便两种环境不会产生行为漂移。
26. 作为维护者，我希望 Nix 日志解析输出结构化事实而非 prodash 类型，以便解析测试保持独立且无需重复解析字符串。
27. 作为维护者，我希望测试验证领域层级、计数和生命周期，而不绑定完整 ANSI 帧，以便升级 prodash 时减少无意义失败。

## 实现决策

- 保留 Pin Progress、Source Progress、Package Progress、Pin Step 与 Global Operation 的现有领域语义。
- Nix activity 只补充当前 Source 或 Package 的详情，不成为一级活动图。
- 直接使用 prodash progress tree 与 line renderer，不增加通用渲染后端抽象。
- 保留 `Progress`、`Reporter` 与 `Operation` 作为领域 API；删除独立的事件队列、自制 Renderer 与活动状态镜像。
- 任务编排不持有 prodash handle。`Reporter` 通过共享短锁注册表按 Pin、Source、Package 和 Global Operation 身份查找节点。
- 注册表是 prodash Item 的唯一长期 owner。完成或失败时先写入消息，再移除 Item，使活动节点从树中删除。
- prodash handle 生命周期内保持稳定，显示名可随版本、步骤、Fetcher、Hash key 和文本详情变化。
- 唯一 `default` Source/Package 继续折叠；多 Source/Package 才创建可见子节点。
- 只有 Pin 与 Global Operation 留下持久消息；Source/Package 失败位置合并到 Pin 结果。
- 版本变化的成功 Pin 使用 success 消息；版本相同的成功 Pin 使用低强调 info 消息；失败使用 failure 消息。
- Nix 日志解析提供 `Text` 或带 current、可选 total、unit、label 的结构化计数事实。
- bytes、Git objects 与 LFS objects 共享结构化计数路径；所有并行传输按当前 Source/Package 聚合。
- 仅当所有并行传输均有 total 时才提供聚合 total；任何进度行都不显示原始 URL。
- line renderer 写 stderr，并自动探测 TTY、尺寸、颜色和 `NO_COLOR`。
- TTY 与非 TTY 共用 line renderer。非 TTY 不显示动态帧，只输出持久消息。
- TTY 使用普通滚动区，不使用 alternate screen；不启用 signal-hook，也不隐藏光标。
- renderer 固定使用 10 FPS、无初始延迟并开启 throughput，不新增 CLI 或环境变量配置。
- prodash Root 的消息缓冲区固定为 1024 条。
- prodash 版本为 31；关闭默认 feature，只启用 progress tree、line renderer、crossterm line backend、自动终端配置、bytes unit 与 human unit。
- prodash 渲染始终是 best-effort；渲染故障只停止可视化，不改变业务结果，也不回退到旧 renderer。
- 不修改 Pins File schema、Nix 配置 DSL、Selection 语义或并发任务数量。

## 测试决策

- 主要 seam 使用现有 CLI 端到端测试：通过 fake `nix` 执行真实 Update/Status，验证非 TTY 持久消息、Pin 成功与失败、退出码以及 Pins File 内容。
- CLI seam 验证渲染故障不改变 Pin 结果或 Pins File；测试使用可控失败 writer，而非依赖真实终端故障。
- 模块 seam 验证 prodash tree 的 Pin/Source/Package 层级、`default` 折叠、动态名称、结构化计数、持久消息和节点移除。
- 模块 seam 验证 renderer 启动、shutdown、Drop fallback 和非 TTY 只输出消息。
- 扩展现有 Nix internal-json 解析测试，覆盖已知/未知总量 bytes、并行 transfer 聚合、Git objects、LFS objects 与文本阶段。
- 继续使用现有进度测试所表达的业务预期，例如更新 Pin 高亮、未变化 Pin 降权、多 Source/Package 树和失败位置。
- 不对空格、颜色、光标控制或完整 ANSI 动态帧制作 golden snapshot。
- 不新增 PTY/E2E 视觉测试；动态 TTY 行为通过 prodash tree 状态与 renderer 生命周期覆盖。

## 范围外

- 完整复刻 nom 的 derivation、dependency、build host 或 Nix activity 图。
- prodash 全屏 TUI、键盘交互、alternate screen 或可折叠操作。
- 通用可切换渲染后端抽象。
- signal-hook、统一取消模型或新的 Ctrl-C 行为。
- 自制 prodash renderer、indicatif fallback 或独立非 TTY renderer。
- 新的 CLI 参数、环境变量或用户可配置刷新率。
- 原始 URL、drvPath 或 Fetcher target 作为 Pin 行名称。
- Pins File、配置 DSL、Checker、Fetcher、Builder 或并发调度语义变更。

## 进一步说明

nom 是交互密度和刷新体验的参考，不是领域模型模板。nix-pins 继续以 Pin 为原子 Update 单元，并使用 prodash 提供的树、计数、消息、吞吐率与 line renderer 能力。当前 ADR-0019 是本规范的架构依据。
