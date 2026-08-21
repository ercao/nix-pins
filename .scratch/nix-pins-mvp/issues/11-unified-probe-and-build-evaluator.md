# 11 — 统一的探测与锁定构建 Evaluator

**构建内容：** 由同一份声明式 `pins-config.nix` 同时产生 nix-pins 需要的 Checker/Intermediate FOD 和用户真正要构建的 packages。Evaluator 内部区分探测与锁定构建模式，但不将模式、Fake Hash 反馈循环暴露给用户。

**被以下阻塞：** 10 声明式 Pin 模块 API

## 验收标准

- [x] Checker 求值只读取上游声明，不需要已存在的 Pins File
- [x] source 探测先以 Fake Hash 构造 Fetcher，并将得到的真实 source hash 写入本轮求值上下文
- [x] `goModules` 和 `npmDeps` 必须使用真实 source hash 对应的 `src`构造，不得把 source hash mismatch 误解析成 `vendorHash` 或 `npmDepsHash`
- [x] 锁定构建模式从 `pins.json` 获得 `version`、`src`、`vendorHash` 和 `npmDepsHash`，构造的 package 不含 Fake Hash
- [x] 用户可以通过一个稳定入口同时传入 `pkgs`、`pins-config.nix` 和 `pins.json`，并获得按 Pin/package 名组织的真实 derivations
- [x] 真实 package 与对应 Intermediate FOD 共用同一份 builder 参数，包括 `modRoot`/`root`、patch、environment 和其他会影响依赖锁定的输入
- [x] 已有 `pins.json` schema 仍作为锁定数据契约，不向 JSON 中写入 Nix 构建逻辑
- [x] CLI seam 测试覆盖 source hash 与 derived hash 不同、同一 Pin 锁定 Go 与 npm 两种依赖，以及二次运行指纹未变时复用已有哈希
- [x] 兼容性取舍被记录：要么提供旧 `{ pkgs, fake, pins }` 配置的迁移诊断，要么明确 schema/API 版本边界
