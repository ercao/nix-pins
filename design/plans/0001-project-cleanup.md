# 项目整体清理

状态：本轮清理完成，当前入口见 README 和 design/tui-examples.md。

ADR 已按当前主题合并为 6 篇；入口为 design/adr/README.md，源码注释和文档引用同步更新。

范围已确认：保留与当前实现一致的代码、领域术语、规格、ADR、验证入口和最新状态截图；删除旧开发记录、迁移备份、过时文档及旧版截图，不归档。

本轮整理重复验证代码、截图输出路径、严格 Clippy 问题和文档引用，保留当前业务行为及其他已有工作区修改。最新验收材料统一生成到 target/tui-validation/latest，临时回放页面自动回收；脚本和说明保留，生成产物本地忽略。

验收包括 Cargo 全目标串行测试、无豁免 Clippy、rustfmt、LSP diagnostics、文档链接、全部状态截图和独立 Pin 流水线 PTY。真实网络构建与系统激活单独界定，不以受控 fixture 代替。

不新增通用清理术语、模块架构或第三方依赖，不创建提交，不应用 Home Manager。
