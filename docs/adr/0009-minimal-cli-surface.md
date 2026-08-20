# CLI 只提供 update 与 status 两个子命令，按名字而非正则过滤

实测迁移目标 `macos-config/pkgs/_sources/generated.json` 有 33 个 Pin 且无一使用 Derived Hash，说明当下的主要动作是批量刷新版本。CLI 因此保持最小：`update` 写入 Pins File，`status` 只读报告（`--refresh` 可联网查最新版本）。过滤以位置参数接受 Pin 名字（`nix-pins update foo bar`），正则退居 `--filter`。

## Considered Options

- 无子命令（nvfetcher 形态）：无法表达「只看不写」这一常用意图。
- 增加 `add` 子命令：需要保留注释与格式的 TOML 编辑器（`toml_edit`），成本只换来少写几行配置，而手写配置更快且更可控。
- 增加 `check` 子命令：与 `status --refresh` 边界模糊。

## Consequences

与 nvfetcher 的 `-f REGEX` CLI 不兼容，迁移需改调用脚本（实测仅一处调用点）。位置参数可支持 shell completion，正则不能。
