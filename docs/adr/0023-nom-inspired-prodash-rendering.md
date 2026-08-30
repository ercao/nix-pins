---
status: superseded by ADR-0024
---

# 使用 prodash 实现 nom 风格的进度显示

nix-pins 保留 prodash line renderer 与现有 `Pin → Source → Package` 进度树，只借鉴 nom 的动态仪表盘语义，不复制 nom 的自定义终端 renderer。TTY 中的 Loading、Resolving、Writing 等过程节点完成后直接消失，Pin 完成后仍留在同一棵树中；唯一 `default` Source 继续折叠，多 Source Pin 保留各 Source 的最终状态，Package 仅在失败时保留对应分支。树使用基于终端高度的软预算：活动和失败 Pin 永不裁剪，成功 Pin 按最近完成优先展示，多 Source Pin 必须作为完整子树一起显示或隐藏，并提示 `showing N of M pins`；总摘要始终统计全部 Pin。这样可以减少过程消息堆积并保留结构化结果，同时继续复用 prodash 的终端适配、进度计数和树渲染；代价是不追求与 nom 固定布局、汇总表和日志区域的像素级一致。

`progress::State` 是全部 Pin 展示状态的唯一来源，prodash `Item` 只作为当前可见节点的投影，可以随裁剪结果删除并重建；这层状态不抽象渲染后端。可见集合在结构或终态变化时重新计算。

该树统一称为 `Pin Progress`，因为层级表达 Pin、Source 与 Package 的领域归属，而不是 Nix derivation 之间的依赖关系；`Dependency Graph` 仅保留给未来可能存在的真实 derivation 图。

Loading、Resolving、Writing 等全局阶段合并到 `Pin Progress · {phase} · showing N of M pins` 标题行，不再创建独立 Operation 节点；结束时标题显示 `Pin Progress · done`。

Pin、Source 与 Package 始终按名称稳定排序，活动或失败优先级只参与软高度预算的可见性选择，不改变节点在树中的相对位置。

终端尺寸在启动时读取，并在 Pin 或 Source 状态变化时顺便重新读取和计算可见集合；不启用 prodash `signal-hook`，也不增加独立的 resize 监听线程。窗口调整会在下一次业务进度更新时生效。

非 TTY、重定向和 CI 使用低频线性日志：每个全局阶段和 Pin 完成时各输出一行，不输出 bytes、objects 等高频进度或 ANSI 树形符号；结束时输出完整错误详情与总摘要。

TTY 结束时先关闭动态 renderer，再从同一个 `progress::State` 输出一次静态最终树，随后输出总摘要；不依赖 prodash 保留已经清除的动态区域。树中的失败节点只显示简短位置；完整错误紧随最终树，集中放在 `Failures:` 区块中，总摘要始终作为最后一行。

节点名称用 `⏸` 表示等待、`⏵` 表示活动、`✔` 表示成功、`⚠` 表示失败。prodash 没有可保留在树中的成功或失败状态，因此不调用会写入 message 的 `done()` / `fail()`；不在名称中手写 ANSI，状态语义不依赖颜色。

Pin 版本文本只在目标版本与当前版本不同时显示 `current → target`；未变化时只显示一次版本，不追加 `done`、`unchanged` 或 `updated`。首次锁定没有当前版本时显示 `— → target`。

在配置加载、Pins File 读取等尚未开始处理 Pin 的全局阶段失败时，TTY 最终只保留 `⚠ Pin Progress · {phase}` 与完整错误，不伪造 Pin 节点，也不输出 `Processed 0 pins`；非 TTY 保持 `nix-pins: <error>` 线性输出。

`Pin Progress` 只用于 `update`；`status` 继续使用即时线性输出，不为其他或未来命令抽象通用 dashboard。

Pin Progress 的软高度预算在可获取终端尺寸时为 `rows / 3`，否则为 20 行，标题行计入预算。活动和失败 Pin 可以突破预算；成功的多 Source Pin 只有完整子树能够放入时才显示。动态树与最终静态树使用同一规则。全部 Pin 可见时标题显示 `N pins`，发生裁剪时显示 `showing N of M pins`；只有一个 Pin 时省略数量。

全局阶段名称固定为 `Loading configuration`、`Checking versions`、`Resolving sources`、`Resolving derived hashes`、`Writing pins.json` 与 `done`；阶段文本本身不重复 Pin 数量。

活动详情只显示在最深的可见节点：Checker 显示在 Pin，Source 工作显示在 Source，Package 哈希显示在 Package；唯一 `default` 层被折叠时，详情上浮到 Pin 或 Source，父节点不重复文本。

不增加 nom 风格的 Builds / Downloads 底部汇总表；bytes、Git objects 与 LFS objects 继续显示在对应活动节点，最终只保留 processed/failed 总摘要。

bytes、Git objects 与 LFS objects 的高频 counter 只原地更新当前可见 prodash Item；只有声明节点、阶段切换、完成或失败等结构变化才重新读取终端尺寸并计算可见集合。

TTY 动态树保持光标可见，不启用 prodash `hide_cursor`，避免未处理的强制终止留下隐藏光标的终端状态。

prodash line renderer 继续使用 10 FPS，不为模仿 nom 的更高帧率增加刷新开销。

节点文本超过终端宽度时按显示列宽截断并追加 `…`。状态符号、节点名与版本优先保留，detail 最先截断；counter 所需宽度予以保留。使用当前依赖图中已有的 `unicode-width` 计算终端宽度，不自行实现字符宽度规则。

状态语义只由 `⏸ / ⏵ / ✔ / ⚠` 符号表达；动态树的颜色交给 prodash 并继续遵守 TTY 与 `NO_COLOR`，不手写成功或失败颜色。

实现集中在 `src/progress.rs`，包含完整展示状态、可见 prodash 投影、高度裁剪、最终快照与相关测试；`src/main.rs` 只把 Operation 调用改为阶段更新并调整最终输出顺序。`Cargo.toml` 直接声明锁文件中已有的 `unicode-width`。不新增模块、自定义 renderer、backend trait 或可切换后端抽象。
