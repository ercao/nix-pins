# CLI 使用 Clap derive，并隔离参数解析与执行

CLI 只提供 update 与 status，未指定子命令时默认执行只读 status；应用配置统一提供全局参数与 NIX_PINS 环境变量，保留 --config、--pins、位置 Pin 名及 --filter。名称与正则取并集，显式名称不存在、非法正则或仅正则却零匹配均报错，不增加 status --refresh、配置自动改写或 nvfetcher 的 -f 兼容别名。

命令解析位于 crates/cli/src/cli.rs，Pin 匹配与名称集合验证位于 selection.rs，配置字段和 Clap 参数元数据只在 settings.rs 的 Settings 中声明，对执行层返回项目自己的 Command 与 Selection；main.rs 不依赖 Clap 类型，避免参数处理侵入任务编排。帮助与版本退出 0，Clap 参数诊断退出 2，不绑定完整排版或 ANSI 颜色，也不维护自定义错误翻译层。

Settings 同时派生 Clap Args 与 Serde Deserialize，保留运行时具体类型；CLI 使用 command_for_update 让配置参数可省略，按字段 id 从 ArgMatches 提取 CLI 或环境变量明确提供的单值参数，再交给 config-rs 合并，未提供的字段不覆盖 TOML。file 字段通过 long = "pins" 保留 CLI 名称，无需单独维护字段映射。运行时 Settings 使用 Serde default 补齐默认值，Clap 不设置参数默认值；help 单独说明默认值，token 的环境变量值隐藏。空环境变量忽略，GITHUB_TOKEN 保留为低于 TOML 的兼容来源。
