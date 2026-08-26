---
status: accepted
---

# 以 Pin 为交互进度单元，并统一解析 Nix JSON 日志

交互式 Update 以 Pin Progress 为一级展示单元，固定显示 Pin 名、Current Version、Target Version 与项目自有的 Pin Step，而不再以全局 Checker、Source Hash、Derived Hash 阶段分组。所有 `nix build` 统一使用 `--log-format internal-json`：TTY 将事件流渲染为动态进度，非 TTY 从同一事件流恢复低频文本与诊断，因此 Hash 提取和错误处理不依赖终端类型。

## Considered Options

- 按全局阶段分组：实现直接，但用户难以追踪单个 Pin 的状态。
- 仅在 TTY 使用 `internal-json`：会产生两套 Nix 日志、Hash 提取与错误路径。
- 直接依赖 `nixcloud/logone`：其解析入口绑定自身终端渲染与全局状态，且没有覆盖所需的 file-transfer、copy-path 与通用逐条事件。
- 以 Pin 分组并统一解析 `internal-json`：界面与领域模型一致，同时保留唯一业务路径。

## Consequences

- TTY 使用终端滚动区内的动态进度，不进入 alternate screen，也不接管键盘输入；完成记录与错误保留在 scrollback 中。
- TTY 直接使用 prodash 的进度树与 line renderer，不增加通用渲染后端抽象；需要替换后端时再重构这条集成边界。
- prodash line renderer 继续写入 stderr，并使用 `auto_configure(StreamKind::Stderr)` 探测 TTY、终端宽度、颜色与 `NO_COLOR`；stdout 不承载交互进度。
- TTY 与非 TTY 共用同一个 prodash line renderer；非 TTY 自动关闭动态帧与颜色，只输出 Pin 和 Global Operation 的持久消息，不保留独立的纯文本 renderer。
- prodash 不启用 `signal-hook`，line renderer 不隐藏光标；Ctrl-C 与 SIGTERM 继续遵循现有进程行为，进度渲染不拥有任务取消语义。
- line renderer 使用 10 FPS、无初始显示延迟并开启 throughput；这些是固定渲染参数，不增加 CLI 或环境变量配置。
- prodash 关闭默认 feature，仅显式启用 `progress-tree`、`render-line`、`render-line-crossterm`、`render-line-autoconfigure`、`unit-bytes` 与 `unit-human`。
- prodash Root 的消息环形缓冲区固定为 1024 条，不增加用户配置；这避免大量 Pin 在相邻渲染帧之间完成时覆盖持久消息。
- `Progress`、`Reporter` 与 `Operation` 继续作为 Pin/Source/Package 进度的领域 API，由其内部直接维护 prodash tree；不再保留独立的 `Event`、`Renderer` 与 `Active*` 状态机，也不向任务编排暴露 prodash handle。
- `Reporter` 克隆共享一个短锁节点注册表，按 Pin、Source、Package 与 Global Operation 身份创建或查找 prodash handle；锁只覆盖注册表访问，节点进度与状态更新交给 prodash。
- 节点注册表是 prodash `Item` 的唯一长期 owner；节点完成或失败时先写入持久消息，再从注册表移除 handle，使 `Drop` 将活动节点从树中删除。
- prodash handle 的身份在任务生命周期内保持稳定，但节点显示名随 Current/Target Version、Pin Step、Fetcher、Hash key 与当前文本详情动态更新；状态变化不重建节点或重置计数。
- 只有 Pin 与 Global Operation 使用 prodash 的 `done`/`fail` 留下 scrollback 消息；Source 与 Package 节点只在活动期间存在，其失败位置归并到所属 Pin 的结果。
- Pin 完成时，Current 与 Target Version 不同使用 prodash `done`，版本相同使用低强调的 `info`，失败使用 `fail`；两种成功消息都表达 Done，不把版本相同解释为没有重算哈希。
- Nix 下载活动向进度层提供结构化字节计数：有总量时包含 current/total，无总量时只包含 current，由 prodash 计算吞吐率与 ETA；stdenv phase、Copying、Building 与 Querying 仍是文本详情，不展开为独立活动节点。
- 同一构建中的并行下载聚合到当前 Source 或 Package 节点：累加已下载字节并显示 transfer 数；只有全部 transfer 都有总量时才提供聚合总量，且永不创建或显示带 URL 的下载子节点。
- `nix.rs` 以小型 `NixProgress` 枚举表达 `Text` 或 `{ current, total, unit, label }` 计数事实，统一覆盖 bytes、Git objects 与 LFS objects；`progress.rs` 再将其映射到 prodash，Nix 日志解析不依赖 prodash 类型，也不重复解析展示字符串。
- 使用现有 `serde_json::Value` 实现最小逐行解析器，忽略未知 action、type 与附加字段；解析问题只能降低展示精度，不能改变任务结果。
- 测试覆盖 prodash tree 中的节点层级、状态、计数、持久消息，以及 renderer 的启动、关闭和非 TTY 输出；不对完整 ANSI 动态帧制作 golden snapshot。
- prodash 渲染是 best-effort 观察层；renderer 的启动、刷新或关闭错误只停止可视化，不改变 Pin 结果、退出码或 Pins File，也不回退到另一套 renderer。
- Pin Step 由 nix-pins 定义；Nix activity 只补充 Downloading、Copying Store Path、stdenv Phase、Building 或 Querying Cache 等详情。
- Pin 行使用声明中的 Pin 名和 Current/Target Version 固定位置，但不显示表头。Target Version 是 Checker 候选，不表示已经提交。
- TTY 只动态显示活跃 Pin，Done 或 Failed 后转为静态行。Failed 行只保留失败位置，完整错误仍由最终摘要输出。
- Done 只表示 Pin 计算完成；持久化由独立的 Writing Pins File Global Operation 表达。批量 Probe 也使用 Global Operation，不伪装成单 Pin Step。
- 同一 Pin 的 file-transfer activity 按 Pin 汇总；fetchurl 的 curl 进度与 fetchgit/Git LFS 的对象进度从 `resBuildLogLine` 提取。只有总量已知时显示 `done/total`。进度行永不显示原始 URL。
- 非 TTY 不输出动态 Pin 行或高频字节更新，但继续保留低频里程碑和完整诊断。
- 当前全局阶段屏障与并发模型保持不变；Pin Progress 只是执行状态的投影，不把 Update 改造成逐 Pin 流水线。
