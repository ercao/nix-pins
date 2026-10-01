# nix-pins 当前项目规格

nix-pins 用 Rust 检查上游版本，通过真实 Nix Fetcher 和 Intermediate FOD 计算哈希，输出 JSON Pins File。公开声明使用 `{ pin }:`，应用运行参数使用 config-rs；领域术语见 [CONTEXT](../../CONTEXT.md)。

## 配置与能力

- Pin 拥有一个 Checker、一个 Version 和一个或多个 Source；Source 拥有 Fetcher、补丁与 Package。Source 和 Package 共用所属 Pin 的 Version。
- `pin.mk` 正交组合 Checker 与 Fetcher；`pin.github`、`pin.git` 提供常见组合的字符串或属性集简写。单 Source 顶层声明规范化为 `sources.default`，显式 `sources` 优先。
- Checker 支持外部命令、GitHub、git、crates.io、PyPI、npm 和 URL 正则。Version 保留原始标识，只清除协议性的首尾空白。
- Git Checker 默认解析 HEAD；branch 和 ref 解析为 commit SHA，tag 模式支持 include/exclude 正则及 semver/字典序选择。无候选属于 Pin Failure。
- HTTP GET 的临时连接错误及 429、500、502、503、504 可重试；最多三次额外重试，默认等待 250ms、500ms、1s，正整数秒 Retry-After 优先且单次最多 30s。解析与业务错误不重试。
- Fetcher 支持 GitHub、git、URL、zip、Hugging Face。Version 映射在 Nix 内求值，不将函数保存到 JSON。
- Package 支持 Go、npm 与 pnpm 依赖产物；Source 的 patches/postPatch 同时作用于下游源码和依赖计算。
- 未知字段、缺失参数、Fetcher 保留字段冲突和不可序列化结果在运行任何 Checker 前作为 Configuration Error 拒绝，不写 Pins File。
- Nix 配置及外部命令属于受信任代码，不提供额外沙箱。
- [应用配置](../../docs/content/zh-CN/3.reference/2.configuration.md) 定义默认值、TOML、环境变量与 CLI 的覆盖顺序。

## CLI 与持久化

`status` 只读报告当前 Version 与上次失败，不运行 Checker；未指定子命令时默认执行 status。`update` 更新选中的 Pin。全局路径参数为 `--config`、`--pins`。

位置 Pin 名与 `--filter` 正则取并集；显式名称不存在、非法正则或仅提供正则却零匹配均报错。完整 Update 清理配置中已不存在的条目，选择性 Update 保留范围外条目。

Pins File 使用 schema v2：`pins.<pin>.version` 与 `pins.<pin>.sources.<source>`，Source 保存 Fetcher、Hash、Derived Hash 与 Fingerprint。空能力整个键缺失，键序稳定。Reader 严格保留 Source 层级，不提升唯一 default Source 的属性；不读取或迁移 schema v1。

写入事务使用排他进程锁、同目录临时文件、同步与原子替换；内容字节不变时不替换。Pin 内任一 Source 失败则保留整 Pin 的旧结果，首次失败不创建不完整条目；其他 Pin 可成功写入，整轮以非零退出码报告部分失败。取消退出 130 且不写文件。

## 求值与调度

Checker 求值先取得声明与配置错误。全部 Checker 完成后，各 Pin 独立进行源码求值、下载、源码 Hash、派生求值及 Derived Hash；快 Pin 不等待其他 Pin 下载完成。

下载默认并发 2；Checker 与 Hash 默认可用逻辑 CPU 数除以 2、向下取整且至少 1。下载与 Hash 队列分离，Nix 求值和派生构建共同使用 Hash 队列。Pin 内 Source 结果原子聚合，最终统一保存。

Source Hash 与 Derived Hash 共用 Fake Hash 反馈循环；只构建对应源码或 Intermediate FOD，不构建完整应用。固定 SRI Fake Hash 用于求得 drvPath 指纹；输入不变时复用 Hash。Nix 日志解析不得以退出码或展示文本推断成功，缺少真实 Hash 时附完整诊断。

## 进度与分发

TTY 使用应用 Ratatui 任务视图，Pin/Source/Package 显示层级、状态、版本和结构化进度。未知总量不画百分比条。动态图标仅属于显示，业务状态不依赖终端刷新。取消、终端恢复、降级、Messages 与最终输出见 [ADR-0026](../adr/0026-application-ratatui-task-view.md)。

Flake 当前提供 aarch64-darwin 的 package、app、check 与 devShell；安装时一同分发 Nix evaluator。Flake package 当前 `doCheck = false`，不能把 `nix flake check` 的构建通过称为 Cargo 测试通过。

## 验证

- 主要边界是完整 CLI：断言 Pins File、退出码、配置错误、部分失败、复用及取消，而非内部调用次序。
- Nix 公共声明及 Reader 求值覆盖 Source/Package 归属、Version 映射和 schema v2。
- 默认 Cargo 测试使用本地桩；真实网络或依赖构建测试按现有 ignored 标记显式运行。
- Ratatui TestBackend 验证布局和窄屏；PTY 验证完整终端生命周期、快慢 Pin 调度与取消。
- 当前命令与最新截图入口见 [README](../../README.md)、[TUI 示例](../tui-examples.md)。

不包含 Rust Builder、schema v1 迁移、自动改写用户声明、status --refresh、插件系统或提交/发布自动化。
