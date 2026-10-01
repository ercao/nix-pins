# pnpm 依赖支持

状态：已实现，完整测试及规范、规格两条审查均已通过。

## 已确认范围

- 参考现有 npm 依赖管理：计算并保存 Derived Hash，导出可复用的 pnpm 依赖产物；更新哈希只构建对应的 Intermediate FOD。
- `pnpm` 为可选参数，省略时使用传入的 `pkgs.pnpm`；需要特定主版本时可显式覆盖，例如 `pnpm = pkgs.pnpm_10`。默认值由调用方使用的 Nixpkgs 决定，不读取机器上全局安装的 pnpm，也不从上游 `package.json` 自动推断，见 [ADR-0003](../adr/0003-fake-hash-feedback-loop.md)。
- 配置必须显式指定依赖抓取格式版本 `fetcherVersion`，沿用底层 `pkgs.fetchPnpmDeps` 的必填约束，nix-pins 不额外提供默认值。该参数控制依赖产物的格式，升级格式后需要重新计算哈希。
- 首版支持普通项目和 workspace，预取所选锁文件的完整依赖，不提供按 workspace 子包过滤的能力。这样先避免共享依赖与过滤规则增加首版复杂度；子包过滤留待后续需求评估。

## 实现范围

锁文件处理沿用上游 frozen-lockfile 行为和现有 Source 补丁入口，不增加自动修复能力。验收使用最小普通项目与包含共享依赖的 workspace 示例。

## 已确认的公开接口

沿用 `pin.npmPackage` 的命名方式，新增 `pin.pnpmPackage`，放在 Source 的 `packages` 中：

```nix
packages.default = pin.pnpmPackage {
  root = ".";
  fetcherVersion = 4;
};
```

`root`、`fetcherVersion` 为必填，示例中的 `fetcherVersion = 4` 不是默认值。`pnpm` 可省略，默认使用 `pkgs.pnpm`；需要指定版本时添加 `pnpm = pkgs.pnpm_10;` 等覆盖。依赖产物名称为 `pnpmDeps`，Derived Hash 基础名称为 `pnpmDepsHash`，具体归属及同类 Package 的名称区分沿用现有规则。

`root` 必须是非空字符串，指向解包后源码中的项目目录；`fetcherVersion` 必须是正整数，其可用值及 pnpm 兼容性由调用方的 Nixpkgs 约束。首版拒绝非空 `pnpmWorkspaces` 和 `pnpmInstallFlags` 中的 workspace 过滤参数。

该 Package 本身承载依赖产物，并通过 `.pnpmDeps` 暴露同一 derivation。下游从 Reader 的 `sources.<source>.packages.<package>.pnpmDeps` 获取缓存，配合相同的 pnpm 包、Node.js 和 `pnpmConfigHook` 使用；子目录项目同时设置下游的 `pnpmRoot`。应用的构建及安装步骤由下游定义。

配置校验将 derivation 作为原子输入，只强制其 `drvPath`，避免在显式传入 pnpm 包时递归求值其 passthru 引用的整个包集合。其他配置值仍在 Checker 执行前检查。

## 底层依赖与版本核对

通过 `pkgs.fetchPnpmDeps` 构造依赖的 Intermediate FOD；下游可通过 Nixpkgs 的 `pnpmConfigHook` 消费该依赖产物。

已核对 macos-config 锁定的 Nixpkgs 提交 `f165e44f135784a494bca3d4ab4834c139f0b37d`：

- [fetchPnpmDeps 实现](https://github.com/NixOS/nixpkgs/blob/f165e44f135784a494bca3d4ab4834c139f0b37d/pkgs/build-support/node/fetch-pnpm-deps/default.nix) 中，`pnpm` 默认使用包集合提供的 pnpm；`fetcherVersion` 虽声明为 `null`，随后会断言非空，省略即报错。
- 该提交支持格式版本 3、4，并拒绝 pnpm 11 搭配格式版本 3。示例因此使用格式版本 4，不能把该支持范围视为所有 Nixpkgs 版本的固定契约。
- 对该提交进行了派生求值检查：省略格式版本失败；默认 pnpm 11 搭配格式 3 失败、搭配格式 4 成功；pnpm 10 搭配格式 3 成功。这些检查未执行依赖下载或构建。

### 锁文件处理

在上述 Nixpkgs 提交中，`fetchPnpmDeps` 使用 `pnpm install --frozen-lockfile` 预取依赖，`pnpmConfigHook` 使用 `pnpm install --offline --frozen-lockfile` 安装依赖，没有自动修复或生成 `pnpm-lock.yaml` 的步骤。

实现中的 `pnpm-fixup-state-db` 处理 pnpm 11 store 的 SQLite 状态数据库，不是锁文件修复器。上游提供 `prePnpmInstall` 入口供调用方自定义安装前操作；锁文件版本检查发生在该入口之前。

现有 Source 的 `patches` / `postPatch` 可以用于预先修补源码中的锁文件，让依赖预取和下游使用同一份修补后的源码。本次实现保留这些入口，不增加专门的修复器。

## 验收

- 公共 Reader 的锁定哈希、默认 pnpm、显式版本覆盖、多 Package 归属，以及 CLI 的配置错误通过常规测试验证。
- 普通项目通过 Update 生成锁定结果，再由独立下游 derivation 离线安装依赖并完成构建。
- workspace 示例在含空格的子目录中，通过 Source 补丁补入锁文件，验证共享依赖和下游离线构建。
- 实际 Update 验证未变化输入的复用、pnpm 版本变更后的重新计算、锁文件失败时整个 Pin 的回滚、其他 Pin 的独立成功，以及首次失败不创建不可构建条目。

涉及工具链构建及注册表下载的三个测试默认标记为 ignored，可用 `cargo test --test pnpm -- --ignored --test-threads=1` 显式运行。它们已在本次实现中逐项执行；不以普通求值测试替代真实构建结果。

最新常规测试与截图入口见 [README](../../README.md)，不保留旧轮次的测试数量作为当前验证结论。

## 现有实现依据

- `nix/builders.nix` 将 `npmPackage` 映射到 `npmDepsHash` 和 `npmDeps`，并要求显式传入 `root`。
- [ADR-0003](../adr/0003-fake-hash-feedback-loop.md) 规定 Derived Hash 通过具名中间产物获取。
- [ADR-0003](../adr/0003-fake-hash-feedback-loop.md) 记录 npm 的锁文件要求及通过补丁提供锁文件的方式；pnpm 的对应契约采用 frozen-lockfile。
