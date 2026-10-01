# 声明式 Nix API 正交组合 Checker 与 Fetcher

公开配置使用 { pin }: 声明，由 evaluator 构造 Fetcher、Fake Hash、锁定值与实际 Package；不兼容直接接收 pkgs、fake、pins 的旧签名。先求值 Checker 声明及配置校验，选定 Version 后再求值 Source 和 Package，避免 Checker 依赖尚未选出的版本或工具改写用户配置。

Checker 决定锁定哪个原始 Version，Fetcher 决定如何取得源码。Checker 在 Rust 中实现 GitHub、git、crates.io、PyPI、npm、URL 正则，并保留外部命令作为长尾入口；不依赖 nvchecker。Version 仅清除协议性的首尾空白，候选选择属于具体 Checker，不进行全局 semver 规范化。

pin.mk 保留任意正交组合；pin.github 与 pin.git 是常见组合的简写，接受字符串或属性集，单 Source 声明规范化为 sources.default，显式 sources 优先。Source 不包含 Checker，Package 共享所属 Pin 的 Version 和所属 Source。公开 target 表达上游目标，原生 Nix 函数映射 Version；映射只在已有版本时求值，结果必须可序列化，函数不进入 Pins File。

未知字段、缺失参数、fetcherArgs 覆盖 owner/repo/url/rev/hash 等保留字段，以及不可序列化结果属于 Configuration Error，在运行任何 Checker 前拒绝且不写文件。外部命令 stdout 必须在清除首尾空白后恰好包含一行非空 Version，诊断走 stderr；声明及命令属于受信任代码，不提供额外沙箱。真实 Package 通过 nix/packages.nix 按 Pin/Source/Package 层级消费锁定结果。
