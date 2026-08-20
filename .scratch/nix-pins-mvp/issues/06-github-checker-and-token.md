# 06 — GitHub Checker 与 token

**构建内容：** 用户以 `check.github = "owner/repo"` 声明上游，工具自动查出最新 release 版本；配置 token 后不再受匿名调用的配额限制。

**被以下阻塞：** 01

## 验收标准

- [x] `check.github` 可用，能查出最新 release 版本
- [x] 支持 token，且 token 不出现在日志或错误输出中
- [x] Checker 的 API base 可覆盖，使其可指向本地桩进行测试
- [x] 触及配额上限时的失败遵循部分失败语义并给出可辨识的提示
- [x] 经 CLI seam 的测试以本地桩覆盖成功与配额耗尽两种路径

## 说明

匿名调用实测 rate limit 为 60/小时，对 33 个 Pin 的仓库一轮即耗去约半数配额，因此 token 不是可选项。

本 ticket 确定 HTTP 客户端与并发运行时的组合选择（同步客户端配线程池，或异步客户端配 tokio）。

## 被以下阻塞

- 01 — update 最小端到端通路
