# 09 — macos-config 33 个 Pin 迁移验证

**构建内容：** 真实仓库的全部 Pin 用本工具跑通，且产出的哈希与 nvfetcher 既有结果逐条一致——证明工具可以取代现有方案而非仅在示例上可用。

**被以下阻塞：** 02、03、06、07、08

## 验收标准

- [x] 33 个 Pin 全部写出配置并成功产出 Pins File
- [x] 每个 Pin 的 src hash 与 nvfetcher 既有结果逐条比对一致
- [x] `github` 与 `git` 两种 Fetcher 在真实数据上均正确
- [x] 重复运行时指纹跳过生效，第二轮不重新取哈希
- [x] 记录一轮全量更新的实际耗时与配额消耗
- [x] 比对中若出现不一致，定位原因并记录为 ADR 或修正

## 被以下阻塞

- 02 — status 与部分失败语义
- 03 — 指纹跳过（src hash）
- 06 — GitHub Checker 与 token
- 07 — 两级并发
- 08 — 其余内置 Checker
