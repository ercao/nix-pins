# pnpm 默认使用调用方的包集合，抓取格式版本显式指定

pnpm 支持沿用现有 npm 依赖的管理方式：计算并保存 Derived Hash，导出可复用的依赖产物，更新哈希时只构建对应的 Intermediate FOD。

pnpm 依赖声明的 `pnpm` 参数可选，默认使用调用方传入的 `pkgs.pnpm`，需要特定主版本时可显式覆盖，例如 `pnpm = pkgs.pnpm_10`。默认选择由调用方使用的 Nixpkgs 决定，避免每份声明重复传入默认工具；更新 Nixpkgs 时默认 pnpm 可能跨主版本变化，有兼容性要求的项目应显式选择。工具不读取机器上全局安装的 pnpm，也不从上游 `package.json` 自动选择版本，避免引入上游版本声明到可用 Nix 包之间的隐式映射规则。

`fetcherVersion` 沿用底层 `pkgs.fetchPnpmDeps` 的必填约束，nix-pins 不额外提供默认值。已核对的 Nixpkgs 提交 `f165e44f135784a494bca3d4ab4834c139f0b37d` 会在省略该参数时直接报错；这不是 nix-pins 为方便管理而额外添加的限制。该版本号用于在抓取实现演进时保持既有输出哈希的兼容性，升级格式需要重新计算哈希。由配置作者显式升级，避免 nix-pins 自选格式引入隐藏的哈希变化。
