# 自行实现版本检查，不依赖 nvchecker

nvfetcher 把版本检查委派给 nvchecker，因此运行时同时需要 Haskell 与 Python 环境。本工具在 Rust 内置实现少量高频 Checker（GitHub release/tag、git ls-remote、crates.io、PyPI、URL 正则），保持单静态二进制分发，并避免让 nvchecker 的 TOML 方言与 keyfile 机制反向绑定本工具的配置格式。

## Considered Options

- 包装 nvchecker：可立即获得 40+ 种上游 source，但放弃单二进制目标，且配置格式受制于 nvchecker。

## Consequences

放弃了 nvchecker 长尾 source 的开箱支持，因此外部命令形式的 Escape Hatch Checker 是必需功能而非可选功能。
