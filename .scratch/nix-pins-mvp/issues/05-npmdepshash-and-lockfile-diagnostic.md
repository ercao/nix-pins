# 05 — npmDepsHash 支持与缺 lockfile 诊断

**构建内容：** npm 包与 Go 包走同一套流程自动算出 `npmDepsHash`；当源码树不含 lockfile 时，用户得到一条指向既定解法的提示，而不是一句无信息的解析失败。

**被以下阻塞：** 04

## 验收标准

- [x] 识别出承载 `npmDepsHash` 的 Intermediate FOD 并构建它
- [x] 同一次求值中 Go 与 npm 两种 builder 各自被正确识别，`derived` 只出现自己那种哈希
- [x] 用户可在配置中以 `postPatch` 引用另一个被 fetch 的 lockfile，该场景端到端可用
- [x] 源码树无 lockfile 导致构建在哈希比对之前失败时，输出一条指向 postPatch 提供 lockfile 这一做法的提示
- [x] 该失败不被误判为哈希解析失败，退出码遵循部分失败语义
- [x] 经 CLI seam 的测试以桩输出覆盖缺 lockfile 的失败模式

## 被以下阻塞

- 04 — vendorHash 支持
