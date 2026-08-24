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

- 使用现有 `serde_json::Value` 实现最小逐行解析器，忽略未知 action、type 与附加字段；解析问题只能降低展示精度，不能改变任务结果。
- Pin Step 由 nix-pins 定义；Nix activity 只补充 Downloading、Copying Store Path、stdenv Phase、Building 或 Querying Cache 等详情。
- Pin 行使用声明中的 Pin 名和 Current/Target Version 固定位置，但不显示表头。Target Version 是 Checker 候选，不表示已经提交。
- TTY 只动态显示活跃 Pin，Done 或 Failed 后转为静态行。Failed 行只保留失败位置，完整错误仍由最终摘要输出。
- Done 只表示 Pin 计算完成；持久化由独立的 Writing Pins File Global Operation 表达。批量 Probe 也使用 Global Operation，不伪装成单 Pin Step。
- 同一 Pin 的 file-transfer activity 按 Pin 汇总；只有全部总量已知时显示 `done/total`。进度行永不显示原始 URL。
- 非 TTY 不输出动态 Pin 行或高频字节更新，但继续保留低频里程碑和完整诊断。
- 当前全局阶段屏障与并发模型保持不变；Pin Progress 只是执行状态的投影，不把 Update 改造成逐 Pin 流水线。
