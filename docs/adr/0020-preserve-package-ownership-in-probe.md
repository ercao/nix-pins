---
status: accepted
---

# Probe 显式保留 Package 与 Derived Hash 的归属

Pin Progress 在 Derived Hash 工作期间使用 Package 树形子节点，因此内部 Probe 结果显式表达 `Package → Derived Hash → drvPath`。不能从 `vendorHash` 或 `api.vendorHash` 等扁平键名反推 Package：单一同类 Package 的 Hash 名没有 Package 前缀，字符串约定也不应成为 UI 归属协议。

该结构只扩展 evaluator 到 Rust 的内部 Probe JSON；Pins File 继续使用既有 Derived Hash 键，公开 Nix 配置 API 也保持不变。Checker、Version 与 Source 属于父级 Pin，只有 Derived Hash 进度属于 Package 子节点。唯一名为 `default` 的 Package 保持内联；仅当存在多个 Package 或唯一 Package 是具名 Package 时展开树，避免常见单 Package 场景额外占用终端行。Package 树只在父级活跃期间显示，完成后折叠成静态 Pin 行；失败行保留 `Package/Derived Hash` 定位，以便用户知道哪个子任务失败。树形展示不改变执行调度：同一 Pin 内的 Derived Hash 保持顺序处理，既有 Hash worker 仍以 Pin 为任务单位。
