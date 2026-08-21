# 12 — CPA-Manager-Plus Go/npm 组合示例

**构建内容：** 新增 `examples/example2`/`CPA-Manager-Plus`示例，用一份声明式配置同时锁定 `apps/manager-server` 的 Go 依赖和根 `package-lock.json` 覆盖的 npm workspace 依赖，并直接暴露可构建的 server/web packages。

**被以下阻塞：** 10 声明式 Pin 模块 API；11 统一的探测与锁定构建 Evaluator

## 验收标准

- [x] 示例只锁定 `https://github.com/seakee/CPA-Manager-Plus`，GitHub Checker 使用 `seakee/CPA-Manager-Plus`
- [x] Go package 指向 `apps/manager-server`，并使用与真实构建相同的 Go module 参数计算 `vendorHash`
- [x] npm package 使用仓库根目录的 `package-lock.json`，锁定其中包含 `apps/web` 的 workspace 依赖
- [x] 如需要 `GOPROXY`，它作为 Go builder 的共享声明输入，不在探测和真实构建中分别维护
- [x] 生成的 `pins.json` 包含经真实构建得到的 source hash、`vendorHash` 和 `npmDepsHash`，三者不得因误解析而被写成同一个 source hash
- [x] 通过统一 evaluator 可获得 manager-server 和 web 的真实 derivations，示例不重复书写 `buildGoModule` 或 `buildNpmPackage`
- [x] 完成一次真实网络锁定验证，记录上游版本及最终三类哈希
- [x] 完成锁定后，至少对 Go/npm 依赖 FOD 进行构建验证，确认不再发生 hash mismatch
