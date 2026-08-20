# Derived Hash 通过构建具名中间 attribute 获取，而非构建整个包

假哈希循环不构建整个包，而是构建 Derived Hash 所属的中间 FOD attribute：`goModules`（buildGoModule）、`npmDeps`（buildNpmPackage）。src hash 同理构建 `.src`。这样构建在拿到哈希后立即停止，不会继续执行编译。

本机 Nix 2.35.2 实测（`nixpkgs` = `/nix/store/lk453zl4b266layvrpg43123ljlnxknm-source`）：

```
curlie 1.8.2  .src        got: sha256-BlpIDik4hkU4c+KCyAmgUURIN362RDQID/qo6Ojp2Ek=   8.0s
curlie 1.8.2  .goModules  got: sha256-GBccl8V87u26dtrGpHR+rKqRBqX6lq1SBwfsPvj/+44=   8.3s
sloc   0.3.2  .npmDeps    got: sha256-cFUWwmsYy75qAfhkY6tc4Hxwjo8WJcC5urQC22UnnVU=  12.3s
```

`sloc` 的 npmDepsHash 与 nixpkgs `pkgs/by-name/sl/sloc/package.nix` 中记录的值逐字符一致，确认该机制产出的哈希正确。

ADR-0003 的「失败的 FOD 仍把输出注册进 store」结论在两种 Derived Hash 上均成立：

```
/nix/store/p4yvq20cwd2ijimank28jca3qhgq33ww-curlie-1.8.2-go-modules -> IN_STORE
/nix/store/q97ladcrdn99pl5n20sb0rzh8gcbym5b-sloc-0.3.2-npm-deps     -> IN_STORE
```

## Consequences

必须为每种 builder 维护「Derived Hash 名 → 中间 attribute 名」的映射（`vendorHash → goModules`、`npmDepsHash → npmDeps`）。这是 ADR-0003 中曾以「依赖 nixpkgs 内部实现细节」为由否决选项的一部分，但此处仅使用 attribute 名而不自行计算哈希，暴露面远小于被否决的方案。

npmDeps 的输入集与 goModules 不同（`forceGitDeps`、`forceEmptyCache`、`src`、`srcs`、`sourceRoot`、`prePatch`、`patches`、`postPatch`、`patchFlags`、`fetcherVersion`），因此 ADR-0004 的白名单必须按 builder 分别定义，不能共用一份。
