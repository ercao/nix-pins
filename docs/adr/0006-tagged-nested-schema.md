# Pins File 采用带标签的嵌套 schema

nvfetcher 的 `generated.json` 把所有 fetcher 与所有扩展能力平铺在同一命名空间，未使用的能力以 `null` 出现在每个条目上（实测 `macos-config/pkgs/_sources/generated.json` 33 个条目均带 `cargoLock: null`、`extract: null`、`passthru: null`）。本工具改为按能力嵌套：每种 fetcher 只出现自身字段，未使用的能力整个键缺失。

```json
{
  "schemaVersion": 1,
  "pins": {
    "agent-browser": {
      "version": "v0.34.0",
      "fetcher": { "github": { "owner": "vercel-labs", "repo": "agent-browser", "rev": "v0.34.0" } },
      "hash": "sha256-UdCBSe7w0ZgJimB7ixGcaabJjH3m6O0vB1SV9n9apfE="
    }
  }
}
```

## Considered Options

- 平铺 schema：`jq` 路径更短，Rust 侧反序列化错误信息更直接；但每新增一种 fetcher 或 Derived Hash 都会给全部既存条目增加 `null` 字段，污染迁移 diff。

## Consequences

Reader 侧成为对 `fetcher` 键的直接分派（`if f ? github then fetchFromGitHub ...`），无需依据 type 字符串猜测哪些字段有效。Rust 侧需使用带标签的 enum 表达 fetcher。
