# 03 — 指纹跳过（src hash）

**构建内容：** 版本与构建输入均未变时，用户重复运行 `update` 不再重新取 src hash。日常更新从每轮全量重取变为只处理真正变化的 Pin。

**被以下阻塞：** 01

## 验收标准

- [x] 以带 Fake Hash 的 drvPath 作为指纹写入 Pins File
- [x] 指纹未变时跳过取哈希，既有哈希被复用
- [x] 构建输入变化导致 drvPath 变化时触发重取
- [x] Fake Hash 取固定 SRI 值——指纹须始终以假哈希求得，不得受真实哈希影响
- [x] 经 CLI seam 的测试覆盖跳过与触发两种路径

## 说明

该指纹由 Nix 依据 derivation 的全部真实输入计算。原型实测其恰好在影响结果的输入变化时变化，在 `ldflags`、`subPackages`、`doCheck` 等不影响的输入变化时保持不变。

## 被以下阻塞

- 01 — update 最小端到端通路
