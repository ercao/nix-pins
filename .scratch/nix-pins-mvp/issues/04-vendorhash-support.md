# 04 — vendorHash 支持

**构建内容：** 用户在配置中用 `buildGoModule` 描述一个 Go 包并把 `vendorHash` 设为 `fake`，运行 `update` 后该哈希被自动算出并写入 Pins File，无需手工跑构建再复制粘贴。

**被以下阻塞：** 01

## 验收标准

- [x] 配置中的 `derive` 函数被求值，识别出承载 `vendorHash` 的 Intermediate FOD
- [x] 构建对象是该 Intermediate FOD 而非整个包——取到哈希后不继续编译
- [x] 算出的哈希写入 Pins File 的 `derived`，其指纹写入 `fingerprints`
- [x] Reader 把 `derived` 中的哈希透传为包的属性
- [x] 用户在配置中书写的任意真实构建参数（`patches`、`proxyVendor`、`modRoot` 等）均生效，无需工具逐项透传
- [x] 经 CLI seam 的测试覆盖 Derived Hash 的写入与指纹跳过

## 被以下阻塞

- 01 — update 最小端到端通路
