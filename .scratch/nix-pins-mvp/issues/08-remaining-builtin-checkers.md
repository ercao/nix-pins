# 08 — 其余内置 Checker

**构建内容：** 用户可用 git tag、crates.io、PyPI 与 URL 正则声明上游，覆盖常见场景而无需借助外部命令。

**被以下阻塞：** 06

## 验收标准

- [x] git tag（`git ls-remote`）Checker 可用
- [x] crates.io Checker 可用
- [x] PyPI Checker 可用
- [x] URL 正则 Checker 可用
- [x] 各 Checker 的失败均遵循部分失败语义
- [x] 各 Checker 均可指向本地桩进行测试

## 被以下阻塞

- 06 — GitHub Checker 与 token
