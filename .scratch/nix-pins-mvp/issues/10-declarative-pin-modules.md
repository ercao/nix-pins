# 10 — 声明式 Pin 模块 API

**构建内容：** 用一层轻量 Nix DSL 取代当前暴露给用户的 `{ pkgs, fake, pins }` 探测契约。用户只声明上游来源和包类型；`fake`、锁定值选择、Fetcher 以及 Intermediate FOD 的构造由 nix-pins 的 Nix library 负责。

**被以下阻塞：** 无

## 验收标准

- [x] `pins-config.nix` 使用 `{ pin }:` 签名，用户配置中不再出现 `pkgs`、`fake`、`pins`、`src` 或 `derive`
- [x] GitHub Pin 可以声明为 `pin.github { owner = "..."; repo = "..."; ... }`，该声明同时提供 Checker 和 Fetcher 所需信息
- [x] Go 包可以声明为 `pin.goModule { root = "..."; ... }`，npm 包可以声明为 `pin.npmPackage { root = "..."; ... }`
- [x] 同一 Pin 可包含多个具名 package，包名在后续真实构建输出中保持稳定
- [x] builder 声明支持透传真实构建需要的参数，且不需要为探测和真实构建各写一份
- [x] 不支持的 fetcher、builder 或缺失必填字段在 Nix 求值阶段给出包含 Pin/package 名的可辨识错误
- [x] DSL 使用独立的 fetcher 和 builder 模块实现，不引入 NixOS Module System
- [x] 有 Nix 求值测试覆盖单个 Go 包、单个 npm 包以及同一 Pin 中同时存在两种包的声明
