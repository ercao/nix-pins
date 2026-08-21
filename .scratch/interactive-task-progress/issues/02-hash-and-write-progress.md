# 02 将进度覆盖 Source、Derived Hash 与 Pins File 写入

**构建内容：** 用户在 Checker 完成后，可以继续看到 Source Hash、Derived Hash 和 Pins File 写入阶段的独立进度；每个阶段只显示当前活跃 Pin，Derived 任务可显示当前 hash 键，最终仍遵守既有并发、部分失败和完整错误语义。

**被以下阻塞：** 01 建立交互式进度渲染并贯通 Checker 阶段

## 来源

- `docs/specs/0002-interactive-task-progress.md`
- Ticket 01 提供的 progress event 与 renderer seam
- ADR-0007：部分失败语义
- ADR-0008：Checker 与 Hash 两级并发

## 实现约束

- 复用 Ticket 01 的具体进度模块和事件 channel，不创建第二套 renderer 或新的显示抽象。
- Source 阶段总数等于需要重算 src hash 的 Pin 数量。
- Derived 阶段总数等于需要重算任意 Derived Hash 的现有 `DerivedTask` 数量；不得为了进度数字拆分任务或改变失败边界。
- Source 与 Derived 阶段并发数继续取 `NIX_PINS_HASH_JOBS`，默认 1；不得改变 `parallel_map` 的队列和结果收集语义。
- `resolve_source` 中现有 `hashing <pin>: src` 动态状态改由进度事件表达。
- `resolve_derived` 中现有 `hashing <pin>: <key>` 动态状态改由进度事件表达；活跃行可以更新为当前 Derived Hash 键。
- 非 TTY 模式继续输出与现有 `hashing ...` 等价的纯文本，不重新设计 CI 日志。
- 任务数为零的 Source 或 Derived 阶段不创建进度条。
- Writing 阶段只表示现有 Pins File 保存动作，固定为一个串行任务；renderer 不得自行读写文件。
- 阶段切换时清除上一阶段动态区域；正常完成和所有错误返回路径都必须关闭 channel 并等待 renderer 退出。
- worker 业务结果继续通过现有 `BTreeMap` 汇总；progress event 不得承载 Pin 数据或控制错误传播。
- 不解析、过滤或丢弃 `nix build` stderr；Fake Hash 的 `got:` 提取和完整原始错误保持不变。
- 不增加真实下载字节、速度、ETA、统一百分比、Nix `internal-json` 解析、CLI 参数或自定义 signal 处理。

## 验收标准

- [ ] 需要重算 src hash 时显示 Source 阶段总进度和当前活跃 Pin；无需重算时不显示该阶段。
- [ ] 需要重算 Derived Hash 时显示 Derived 阶段总进度和当前活跃 Pin；活跃行能表达当前 hash 键。
- [ ] 一个 Pin 含多个 Derived Hash 时仍只占用一个现有 `DerivedTask` 计数单位。
- [ ] Source 与 Derived 同时显示的活跃任务不超过 `NIX_PINS_HASH_JOBS`。
- [ ] Pins File 保存时显示一次 Writing 阶段，保存完成后清除。
- [ ] 阶段切换、成功退出和失败退出后均不遗留 spinner 或动态行。
- [ ] 非 TTY 不含 ANSI，并保留现有纯文本 hashing 与失败诊断能力。
- [ ] 现有 `NIX_PINS_HASH_JOBS=2` 测试证明进度集成没有串行化 worker。
- [ ] 现有部分失败测试继续证明成功 Pin 写入、失败 Pin 保留旧条目且退出码非零。
- [ ] Nix、HTTP、Git 与 checker 的完整错误正文仍出现在最终失败摘要中。
- [ ] 不新增 Tokio、Nix 日志解析、下载字节进度、新 CLI 参数或详细更新统计。
- [ ] `cargo fmt --check` 与 `cargo test` 通过。

## 被以下阻塞

- 01 建立交互式进度渲染并贯通 Checker 阶段。
