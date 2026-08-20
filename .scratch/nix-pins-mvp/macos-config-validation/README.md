# macos-config 迁移验证

验证日期：2026-08-20

## 固定版本兼容性

- 输入：`/Users/ercao/codes/macos-config/pkgs/_sources/generated.json`
- Pin：33（18 GitHub、10 git、5 URL）
- 配置：`pins-config.nix`；用 nvfetcher 当前版本生成 `check.cmd`，隔离验证 Fetcher 与 src hash
- 全量 warm-store 更新：`real 241.92s`、`user 1.73s`、`sys 0.94s`
- 第二轮指纹跳过：0 个 hash 需要重算，`real 1.06s`
- 与 nvfetcher 当前 `src.sha256` 逐条对比：33/33 一致
- GitHub API 配额消耗：0

## 真实 Checker 全量更新

- 配置：`pins-config-real-checkers.nix`
- Checker：22 个 `github`，11 个动态 `cmd`；后者读取 git 分支 HEAD，并保留失败退出码、最多重试 3 次
- 干净运行：`real-checker-clean-run/` 从无 `pins.json` 开始，报告 33 个 hash 需要重算
- 结果：33/33 Pin、0 failures，`real 323.96s`、`user 2.29s`、`sys 1.35s`
- 完整 warm run：0 个 hash 需要重算、0 failures，`real 8.06s`
- GitHub core 配额观测：5000 初始；干净全量运行前后由 4904 降至 4880。配置本身包含 22 个 GitHub Checker，观测值还包含配额查询等请求开销
- 24 个版本未变化的 Pin，其 hash 与 nvfetcher 当前结果 24/24 一致
- 9 个 Pin 已随真实上游变化：`caveman`、`cli-proxy-api-management-center`、`context7`、`cpa-usage-keeper`、`dws`、`mattpocock-skills`、`metacubexd`、`mihomo`、`models.dev`
- 干净运行结果与前一轮最终 `pins.json` 逐字节一致

`macos-config` 工作树原本已有修改，本验证没有写入该仓库。所有配置、Pins File 与日志均保存在本目录。
