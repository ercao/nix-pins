# 独立 Pin 流水线使用分离队列，运行配置统一加载

全部 Checker 完成后，各 Pin 独立进行源码求值、下载、Source Hash、派生求值和 Derived Hash；一个 Pin 的 Source 齐备后即可推进，不等待其他 Pin 下载完。Pin 内结果原子聚合，最终统一保存 Pins File，保持 [Pin 原子边界](0002-json-lockfile-output.md)。

下载默认并发 2；Checker 与 Hash 默认可用逻辑 CPU 数除以 2、向下取整且至少 1。下载队列与 Hash 队列分离，Nix 求值和派生构建共同受 Hash 并发限制，避免网络等待与昂贵构建共用单一限额；同一 Source 的派生 Hash 在对应任务内顺序处理。

应用参数统一通过 config-rs 加载，优先级为默认值、可选 nix-pins.toml、NIX_PINS 环境变量、CLI；GITHUB_TOKEN 仅作为更低优先级的兼容来源，空环境变量忽略，非法并发值在启动 Nix 前报错，凭证不进入调试输出。Pin 的 Nix 声明独立于应用参数，完整字段见 [应用配置](../../docs/content/zh-CN/3.reference/2.configuration.md)。
