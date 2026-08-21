# pins-config 使用声明式 API v2

status: accepted

`pins-config.nix` 的公开签名从 `{ pkgs, fake, pins }` 切换为 `{ pin }`。用户只声明 `pin.github` 上游和具名的 `pin.goModule` / `pin.npmPackage`；Fetcher、Fake Hash、锁定值与真实 package 由 `nix/evaluator.nix` 统一构造。

这是配置 API v2 的明确版本边界。旧签名不提供兼容层；传入旧配置时，Nix 会报告缺少 `pkgs` 参数。Pins File 继续使用 `schemaVersion = 1`，因为 JSON 数据契约未改变。

真实构建通过 `nix/packages.nix` 接收 `pkgs`、`pins-config.nix` 路径和 `pins.json` 路径，并返回按 Pin/package 名组织的 derivations。

## Considered Options

- 同时支持两种函数签名：需要在 Nix 中探测函数参数并维护两套 evaluator，增加长期分支。
- 修改 Pins File schema：配置 API 变化不改变锁定数据的字段或含义，没有迁移价值。

## Consequences

ADR-0013 与 ADR-0015 中由用户直接接收 `pkgs`、`fake`、`pins` 的配置契约被本决策取代；两阶段求值和 Pins File 单一事实来源仍保留在 evaluator 内部。
