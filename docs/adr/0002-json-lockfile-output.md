# 输出 JSON 锁文件，不生成 Nix 表达式

nvfetcher 生成 `_sources/generated.nix`，使工具必须在宿主语言里拼接发散的 fetcher 调用代码，且生成结果难以 review 与合并。本工具只输出 `pins.json`，并附带一个薄的 `pins.nix` reader（`builtins.fromJSON` 加 fetcher 分派）由用户 import。

## Considered Options

- 生成 Nix 表达式：用户少一步 import，但需在 Rust 内维护 Nix 代码生成，且生成代码在 diff 与冲突解决中是噪音。
- 同时生成两种输出：两份契约会漂移，维护成本翻倍。

## Consequences

`pins.json` 的 schema 与 `pins.nix` reader 共同构成公共 API，必须同步演进并支持 schema 版本迁移。
