# 当前架构决策

只保留当前实现的决策，按主题合并，编号继续用于稳定引用。

| 决策 | 范围 |
| --- | --- |
| [ADR-0002](0002-json-lockfile-output.md) | JSON、schema v2、Pin/Source 边界、部分失败与原子保存 |
| [ADR-0003](0003-fake-hash-feedback-loop.md) | Fake Hash、Intermediate FOD、指纹复用、Package 归属、npm/pnpm |
| [ADR-0008](0008-two-tier-concurrency.md) | 独立 Pin 流水线、并发队列、config-rs |
| [ADR-0016](0016-declarative-config-api-v2.md) | 声明式 Nix API、Checker/Fetcher、Version、便捷构造器 |
| [ADR-0018](0018-clap-compatible-cli-parsing.md) | CLI、Clap、Selection 与解析边界 |
| [ADR-0026](0026-application-ratatui-task-view.md) | Ratatui、Nix JSON 日志、状态树、取消和降级 |
| [ADR-0027](0027-organize-application-modules.md) | 应用模块职责、流水线、进程控制与展示依赖 |
