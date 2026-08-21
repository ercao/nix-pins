# 01 建立交互式进度渲染并贯通 Checker 阶段

**构建内容：** 用户在交互式终端运行 `nix-pins update` 时，可以看到 Checker 阶段的总进度和当前活跃 Pin；阶段结束后动态行被清理，命令最终报告处理数量与失败数量。非交互式调用继续输出纯文本且不含 ANSI 控制字符。

**被以下阻塞：** 无——可立即开始

## 来源

- `docs/specs/0002-interactive-task-progress.md`
- ADR-0007：部分失败语义
- ADR-0008：Checker 与 Hash 两级并发
- ADR-0009：最小 CLI 表面

## 实现约束

- 在 `Cargo.toml` 中加入 `indicatif` 0.18 系列，不引入 Tokio 或其他 async runtime。
- 新增具体的终端进度模块，建议为 `src/progress.rs`；不得增加只有一个实现的 trait 或 factory。
- 使用 `std::io::IsTerminal` 检测 stderr。
- TTY 模式使用唯一的 `indicatif::MultiProgress` renderer；renderer 独占动态 stderr 区域。
- worker 只能通过 `std::sync::mpsc` 发送结构化事件，不得直接操作进度条。
- 事件至少覆盖 Checker 阶段开始、Pin 开始、Pin 完成、阶段完成和全部结束。
- Checker 总数等于本轮选中的 Pin 数量，并发数继续取 `NIX_PINS_CHECKER_JOBS`，默认 8。
- 同时显示的活跃 Pin 不得超过实际 Checker worker 数量；Pin 完成后清除对应活跃行并递增总进度。
- 阶段结束后清除 Checker 动态区域，不留下未完成 spinner 或半行输出。
- 非 TTY 模式不得创建 `MultiProgress`，不得输出 ANSI，并保留现有纯文本诊断。
- 为最终计数可以把 `update` 的内部返回值扩展为只包含 `processed` 与原失败表的结果结构；不得增加版本变化或 hash 复用统计。
- Pins File、退出码、失败排序、完整错误文本和部分失败写入语义保持不变。
- 本 ticket 不为 Source、Derived 或 Writing 阶段增加动态进度；这些由 Ticket 02 完成。

## 验收标准

- [ ] TTY 模式下，Checker 阶段显示独立总进度条与当前活跃 Pin。
- [ ] Checker 任务完成后活跃行被清除，总进度准确递增到选中 Pin 总数。
- [ ] Checker 阶段完成后动态区域被清理。
- [ ] 命令结束时报告处理数量与失败数量，但不增加 `updated/unchanged/hash reused` 分类。
- [ ] 非 TTY 的 stderr 不包含 ANSI escape，并保留现有失败摘要。
- [ ] `NIX_PINS_CHECKER_JOBS` 的含义和默认值保持不变。
- [ ] 单个 Pin 失败时，成功 Pin 仍写入、失败 Pin 保留旧条目、退出码仍为非零。
- [ ] 使用 `indicatif::InMemoryTerm` 留下一个最小 renderer 测试；只断言必要语义，不保存完整 ANSI 快照。
- [ ] CLI seam 测试覆盖非 TTY 无 ANSI 和失败摘要未回归。
- [ ] `cargo fmt --check` 与 `cargo test` 通过。

## 被以下阻塞

- 无——可立即开始。
