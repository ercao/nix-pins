# 01 — update 最小端到端通路（Escape Hatch checker + src hash + Reader）

**构建内容：** 用户写一份只含单个 Pin 的 Nix 配置，用外部命令声明版本来源，运行 `nix-pins update`，得到一份 Pins File；再用随工具分发的 Reader 从该文件求出可用的 `src`。这是第一颗贯穿全层的子弹。

**被以下阻塞：** 无——可立即开始

## 验收标准

- [x] 配置以 `{ pkgs, fake, pins }` 为签名，`check.cmd` 声明版本来源，`src` 中 `rev` 引用 `pins.<name>.version`
- [x] 首轮运行（Pins File 不存在）能完成两阶段求值：阶段一以空 `pins` 只读 `check`，阶段二注入版本后读 `src` 的 drvPath
- [x] src hash 经 Fake Hash 循环取回，写入 Pins File
- [x] Pins File 为 JSON，键序稳定，未使用的能力键缺失且输出不含 null
- [x] Reader 能把该 Pins File 转为可用的 `src`，`github` 与 `git` 两种 Fetcher 均正确分派
- [x] CLI 进程边界的测试 seam 就位：PATH 前置 `nix` 桩脚本，测试不联网且可确定地构造 eval 与 hash mismatch 输出
- [x] 存在一个经该 seam 的测试，覆盖首轮运行产出可构建的 Pins File

## 说明

选 `cmd` 作为首个 Checker 是为了绕开尚未决定的 HTTP 客户端选择，使第一颗子弹能真正穿透全层。

## 被以下阻塞

- 无——可立即开始
