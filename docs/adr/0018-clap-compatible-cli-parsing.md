---
status: accepted
---

# 使用 Clap derive 替换手写参数解析，并保持现有命令兼容

CLI 参数改由 Clap 4 的 derive API 解析，以类型表达 `update`、`status`、位置 Pin 名与选项。迁移保持现有命令语义：未指定子命令时仍执行 `status`；同时提供 Clap 的标准 `-h`/`--help` 与 `-V`/`--version`。未知选项、未知子命令和缺少选项值采用 Clap 的标准诊断并保持退出码 2，帮助与版本输出退出码 0；测试断言关键语义，不锁定颜色或整段排版。暂不维护中文错误翻译层。

迁移只覆盖当前可执行契约，不顺带实现 ADR-0009 已延期的 `status --refresh`；也不为静态命令树使用更冗长的 builder API。

Pin 选择继续以位置名称为主，并保留长选项 `--filter <REGEX>`；不恢复 nvfetcher 兼容的 `-f`。名称与正则同时提供时维持现有并集匹配语义。

Clap 的 derive 类型提取到 `src/cli.rs`，使 `main.rs` 不直接承载命令声明。该模块同时负责编译和验证 `--filter` 正则，对外只返回项目自己的 Command 与 Selection；`main.rs` 不依赖 Clap 类型，也不处理参数验证。虽然当前命令树很小，这个边界仍被有意保留，以隔离参数解析与执行编排。

解析契约由 `cli.rs` 的 `try_parse_from` 级测试覆盖，包括默认 Status、两个子命令、Selection 并集、非法正则与未知参数；二进制 CLI 测试只增加 help、version 与错误退出码 smoke tests。测试不锁定整段 Clap 排版或 ANSI 颜色。

依赖使用 `clap = { version = "4.5", features = ["derive"] }` 并保留默认的 help、usage、suggestions、color 与 error context，由 Cargo.lock 固定实际版本。本次不增加 shell completion、自定义 formatter 或其他 CLI 依赖。
