# `nixcloud/logone` 解析 Nix `internal-json` 的适用性

调查日期：2026-08-24。logone 源码引用固定在仓库提交 [`717f7d774e31147b8ed9159b27034b001a749fde`](https://github.com/nixcloud/logone/tree/717f7d774e31147b8ed9159b27034b001a749fde)；Nix 协议引用固定在 Nix 2.35.2 对应提交 [`2c73b59da29606068c0c98db015dd3a66955525d`](https://github.com/NixOS/nix/tree/2c73b59da29606068c0c98db015dd3a66955525d)。

## 包与维护状态

- logone 是 Rust 2021 项目，同时发布同名 library crate 和 `logone` 可执行文件。README 给出的命令行集成方式是把 `nix build --log-format internal-json` 的输出通过管道送入 `logone --json`；library 根模块公开 `logone`、`parser` 和 `sinks` 模块。[Cargo.toml 第 1–21 行](https://github.com/nixcloud/logone/blob/717f7d774e31147b8ed9159b27034b001a749fde/Cargo.toml#L1-L21)；[README 第 73–78 行](https://github.com/nixcloud/logone/blob/717f7d774e31147b8ed9159b27034b001a749fde/README.md#L73-L78)；[lib.rs 第 1–6 行](https://github.com/nixcloud/logone/blob/717f7d774e31147b8ed9159b27034b001a749fde/src/lib.rs#L1-L6)
- crates.io 在调查日列出的最新版本是 `logone 0.2.9`，发布时间为 2026-02-11，crate 压缩包大小为 361,585 字节，且同时包含 library 和 binary。[crates.io `logone` 元数据](https://crates.io/api/v1/crates/logone)
- 许可证声明为 `MIT OR Apache-2.0`，仓库同时包含两个许可证文件。[Cargo.toml 第 8 行](https://github.com/nixcloud/logone/blob/717f7d774e31147b8ed9159b27034b001a749fde/Cargo.toml#L8)；[README 第 108–112 行](https://github.com/nixcloud/logone/blob/717f7d774e31147b8ed9159b27034b001a749fde/README.md#L108-L112)
- GitHub 仓库未归档。官方提交接口在调查日返回 34 个提交：首个提交日期为 2025-09-09，最新提交日期为 2026-02-18；仓库元数据的最近一次 push 也是 2026-02-18。[GitHub 仓库元数据](https://api.github.com/repos/nixcloud/logone)；[GitHub 提交元数据](https://api.github.com/repos/nixcloud/logone/commits?per_page=100)

## 公开集成 API

- 单行入口是 `parse_nix_line(line: &str, logone: &mut LogOne) -> anyhow::Result<()>`；另一个公开入口 `process_event` 同样接收 `&mut LogOne`，不返回结构化事件。[parser.rs 第 129–184 行](https://github.com/nixcloud/logone/blob/717f7d774e31147b8ed9159b27034b001a749fde/src/parser.rs#L129-L184)；[parser.rs 第 211–218 行](https://github.com/nixcloud/logone/blob/717f7d774e31147b8ed9159b27034b001a749fde/src/parser.rs#L211-L218)
- `LogOne` 同时持有日志缓冲区、状态、终端绘制状态和 derivation 映射；其绘制、消息输出和日志刷新直接写 `stdout`，`Drop` 还会调用 `shutdown`。[logone.rs 第 41–59 行](https://github.com/nixcloud/logone/blob/717f7d774e31147b8ed9159b27034b001a749fde/src/logone.rs#L41-L59)；[logone.rs 第 139–240 行](https://github.com/nixcloud/logone/blob/717f7d774e31147b8ed9159b27034b001a749fde/src/logone.rs#L139-L240)；[logone.rs 第 243–331 行](https://github.com/nixcloud/logone/blob/717f7d774e31147b8ed9159b27034b001a749fde/src/logone.rs#L243-L331)
- CLI 使用 `BufReader::lines()` 逐行调用解析函数，因此可以流式消费；它会静默忽略 `parse_nix_line` 返回的所有错误。[main.rs 第 33–47 行](https://github.com/nixcloud/logone/blob/717f7d774e31147b8ed9159b27034b001a749fde/src/main.rs#L33-L47)
- parser 使用 `serde_json::Value` 而非固定反序列化结构，因此额外 JSON 字段不会导致反序列化失败；但顶层必须是对象并包含字符串 `action`。未匹配的 `(action, type)` 分支直接忽略。[parser.rs 第 129–152 行](https://github.com/nixcloud/logone/blob/717f7d774e31147b8ed9159b27034b001a749fde/src/parser.rs#L129-L152)；[parser.rs 第 211–395 行](https://github.com/nixcloud/logone/blob/717f7d774e31147b8ed9159b27034b001a749fde/src/parser.rs#L211-L395)
- 解析状态包含进程级全局数据：derivation tracking 使用 `OnceLock<Mutex<...>>`；build statistics 使用四个 `static mut` 计数器并在 `unsafe` 块中读写。[parser.rs 第 12–30 行](https://github.com/nixcloud/logone/blob/717f7d774e31147b8ed9159b27034b001a749fde/src/parser.rs#L12-L30)；[nix_build_statistics.rs 第 7–12 行](https://github.com/nixcloud/logone/blob/717f7d774e31147b8ed9159b27034b001a749fde/src/sinks/nix_build_statistics.rs#L7-L12)；[nix_build_statistics.rs 第 54–65 行](https://github.com/nixcloud/logone/blob/717f7d774e31147b8ed9159b27034b001a749fde/src/sinks/nix_build_statistics.rs#L54-L65)

## action、activity 与 result 覆盖范围

Nix 2.35.2 定义的 activity type 包括 `actCopyPath=100`、`actFileTransfer=101`、`actBuilds=104`、`actBuild=105`、`actSubstitute=108` 等；result type 包括 `resBuildLogLine=101`、`resSetPhase=104`、`resProgress=105` 和 `resSetExpected=106`。[Nix `logging.hh` 第 16–43 行](https://github.com/NixOS/nix/blob/2c73b59da29606068c0c98db015dd3a66955525d/src/libutil/include/nix/util/logging.hh#L16-L43)

logone 的显式路由如下：[parser.rs 第 220–395 行](https://github.com/nixcloud/logone/blob/717f7d774e31147b8ed9159b27034b001a749fde/src/parser.rs#L220-L395)

| 输入 | logone 行为 | 是否向调用方暴露结构化数据 |
| --- | --- | --- |
| `start`, type `104` (`actBuilds`) | 注册全局 build-statistics ID | 否 |
| `result`, type `105` (`resProgress`) | 送入 build-statistics handler | 否 |
| `start`, type `105` (`actBuild`) | 创建 derivation 日志缓冲区 | 否 |
| `result`, type `101` (`resBuildLogLine`) | 把 `fields[0]` 存为 `NixMessage.content` | 仅可从公开缓冲区或查询函数读取 |
| `result`, type `104` (`resSetPhase`) | 把 `fields[0]` 转为 `Phase: ...` 后存入缓冲区 | 仅可从公开缓冲区或查询函数读取 |
| `msg` | 按 log level 过滤并直接输出 | 否 |
| `stop` | 仅处理已登记的 statistics ID 或日志 buffer ID | 否 |
| 其他组合 | 忽略 | 否 |

### 下载与复制进度

- Nix 为单个下载创建 `actFileTransfer=101` activity，URL 位于 activity fields；curl 回调通过该 activity 的 `progress(dlnow, dltotal)` 产生 `resProgress=105`。[Nix `filetransfer.cc` 第 467–489 行](https://github.com/NixOS/nix/blob/2c73b59da29606068c0c98db015dd3a66955525d/src/libstore/filetransfer.cc#L467-L489)；[Nix `logging.hh` 第 219–227 行](https://github.com/NixOS/nix/blob/2c73b59da29606068c0c98db015dd3a66955525d/src/libutil/include/nix/util/logging.hh#L219-L227)
- logone 不处理 `start` type `101`，也没有 file-transfer event 类型。它把所有 `result` type `105` 送入 build-statistics handler，而该 handler 只接受先由 `start` type `104` 注册过的 ID；其他 ID 返回 `Unknown id in stats update`。[parser.rs 第 220–228 行](https://github.com/nixcloud/logone/blob/717f7d774e31147b8ed9159b27034b001a749fde/src/parser.rs#L220-L228)；[nix_build_statistics.rs 第 14–50 行](https://github.com/nixcloud/logone/blob/717f7d774e31147b8ed9159b27034b001a749fde/src/sinks/nix_build_statistics.rs#L14-L50)
- 因此公开 API 不会提供 file-transfer 的 `done`、`total`、URL 或 parent activity。CLI 还会静默丢弃上述 handler 错误。[main.rs 第 40–44 行](https://github.com/nixcloud/logone/blob/717f7d774e31147b8ed9159b27034b001a749fde/src/main.rs#L40-L44)
- logone 没有 `actCopyPath=100` 或 `resSetExpected=106` 的显式分支，因此不暴露 copy-path 及其预期字节信息。[parser.rs 第 220–395 行](https://github.com/nixcloud/logone/blob/717f7d774e31147b8ed9159b27034b001a749fde/src/parser.rs#L220-L395)

### build、phase 与原始构建日志

- Nix 的 `actBuild=105` fields 包含 `drvPath`、构建机器名、`1`、`1`。[Nix `derivation-building-goal.cc` 第 697–713 行](https://github.com/NixOS/nix/blob/2c73b59da29606068c0c98db015dd3a66955525d/src/libstore/build/derivation-building-goal.cc#L697-L713)
- logone 的 build-start handler 不读取 `fields[0]`；它读取格式化后的 `text`，并把该文本映射到 ID。因此没有独立的 `drvPath` 字段或 build event 返回值。[nix_logs.rs 第 7–27 行](https://github.com/nixcloud/logone/blob/717f7d774e31147b8ed9159b27034b001a749fde/src/sinks/nix_logs.rs#L7-L27)
- `resSetPhase=104` 被保存为 `NixMessage { message_type: Some(104), content: "Phase: ..." }`；`resBuildLogLine=101` 的 `fields[0]` 被原样保存为 `NixMessage.content`。[nix_logs.rs 第 30–96 行](https://github.com/nixcloud/logone/blob/717f7d774e31147b8ed9159b27034b001a749fde/src/sinks/nix_logs.rs#L30-L96)
- `query_logs_by_id` 可以克隆仍存在的日志缓冲区；`LogOne::print_log_buffer` 会移除缓冲区。因此 library 调用方可以在缓冲区尚未刷新时读取包含 `got:` 的 build log 文本，但 logone 没有逐条日志回调，也不会在解析结果中返回原始文本。[nix_logs.rs 第 134–140 行](https://github.com/nixcloud/logone/blob/717f7d774e31147b8ed9159b27034b001a749fde/src/sinks/nix_logs.rs#L134-L140)；[logone.rs 第 243–291 行](https://github.com/nixcloud/logone/blob/717f7d774e31147b8ed9159b27034b001a749fde/src/logone.rs#L243-L291)

## 错误策略

- 非 `@nix ` 行返回成功并忽略；非法 JSON、非对象、缺少字符串 `action` 以及已处理分支所需字段缺失会返回 `Err`。[parser.rs 第 129–152 行](https://github.com/nixcloud/logone/blob/717f7d774e31147b8ed9159b27034b001a749fde/src/parser.rs#L129-L152)；[nix_build_statistics.rs 第 27–51 行](https://github.com/nixcloud/logone/blob/717f7d774e31147b8ed9159b27034b001a749fde/src/sinks/nix_build_statistics.rs#L27-L51)
- CLI 对这些错误不记录、不退出，继续读取下一行。[main.rs 第 36–47 行](https://github.com/nixcloud/logone/blob/717f7d774e31147b8ed9159b27034b001a749fde/src/main.rs#L36-L47)
- library 调用方可以自行处理 `Result`，但错误值不携带原始结构化事件；未知分支则返回成功并丢弃事件。[parser.rs 第 211–395 行](https://github.com/nixcloud/logone/blob/717f7d774e31147b8ed9159b27034b001a749fde/src/parser.rs#L211-L395)

## 依赖体积

- `logone 0.2.9` 声明 9 个无条件 normal dependencies：`serde`、`serde_json`、`clap`、`crossterm`、`console`、`regex`、`chrono`、`thiserror`、`anyhow`；没有 feature 用于关闭 CLI 或终端渲染依赖。[Cargo.toml 第 23–36 行](https://github.com/nixcloud/logone/blob/717f7d774e31147b8ed9159b27034b001a749fde/Cargo.toml#L23-L36)；[crates.io 0.2.9 dependency 元数据](https://crates.io/api/v1/crates/logone/0.2.9/dependencies)
- crates.io 报告的 0.2.9 压缩包大小为 361,585 字节。[crates.io `logone` 元数据](https://crates.io/api/v1/crates/logone)

## 结论与最小集成建议

**结论：不适合直接作为 nix-pins 的 `internal-json` 解析层。** 它适合把 Nix 日志过滤、缓冲并渲染成较简洁的终端输出，但当前公开 API 不提供 nix-pins 所需的通用 activity/result 事件流，尤其不提供 file-transfer `done/total`、copy-path、结构化 `drvPath` 或 parent 关联；解析状态还与 `LogOne` 的终端输出和全局状态绑定。

最小集成是继续使用 nix-pins 已需的逐行读取，并直接用 `serde_json::Value` 解码 `@nix ` 后的对象，只处理：

1. `start`/`stop` 的 `actFileTransfer=101`、`actCopyPath=100`、`actBuild=105`；
2. `resProgress=105`、`resSetExpected=106`、`resSetPhase=104`、`resBuildLogLine=101`；
3. 未知 action/type 保留为忽略分支，非法行保存原文用于诊断；
4. `resBuildLogLine.fields[0]` 同时送入现有 `got:` 提取逻辑。

这条路径不需要引入 logone 的终端渲染依赖或适配其全局状态，并保留每个 `nix build` 子进程到 Pin 的既有归属关系。
