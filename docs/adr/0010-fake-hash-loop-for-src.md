# src hash 与 Derived Hash 共用假哈希循环，不使用 prefetch 工具

src hash 不通过 `nix-prefetch-git` / `nix-prefetch-url` 获取，而是复用 ADR-0003 的假哈希循环：构造 `fetchFromGitHub { ... hash = FAKE; }` 并解析 `got:`。工具因此只有一条取哈希的代码路径与一套失败模式。

得到的哈希是 nixpkgs fetcher 实际产出的哈希。`fetchSubmodules`、`leaveDotGit`、`sparseCheckout` 等参数的语义只有真实 fetcher 知道；`nix-prefetch-git` 与 `fetchgit` 是两份独立实现，历史上对 `leaveDotGit` 的结果曾不一致。

## Considered Options

- `nix-prefetch-git` / `nix-prefetch-url`（nvfetcher 的做法）：本机可用且输出 JSON，但需模拟真实 fetcher 的参数语义。
- `nix store prefetch-file` / `builtins.fetchTree`：本机实测两者均带 experimental 警告，`nix store prefetch-file` 明确标注接口可能变更。
- 在 Rust 内实现 NAR 序列化自行计算：对 git fetcher 的 submodule 与 leaveDotGit 语义基本不可行。

## Consequences

哈希不匹配失败的 FOD 仍会把正确哈希对应的输出注册进 store（见 ADR-0003 实测），因此后续以真实哈希构建不会重新下载。但该 store path 未被 GC root 保护。
