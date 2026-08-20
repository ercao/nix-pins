# 通过假哈希反馈循环获取 Derived Hash

vendorHash、cargoHash、npmDepsHash 无法从源码 tarball 推导，必须真实执行一次依赖 vendor 过程。本工具采用 nix-update 的做法：以假哈希构造表达式并执行 `nix build`，从失败输出中解析 `got:` 行得到真实哈希。这样工具无需理解任何语言生态的 vendor 机制，新增一种 Derived Hash 类型仅需配置中多一个字段名。

## Considered Options

- 直接构建中间 derivation（`goModules`、`cargoDeps`、`npmDeps`）后 `nix hash path`：依赖 nixpkgs 内部 attribute 路径，漂移更隐蔽。
- 完全外置为用户自定义命令：等同于放弃 Derived Hash 支持这一核心能力。

## Consequences

工具依赖 Nix 的错误输出格式，这是非契约化接口。缓解措施：解析器隔离为单独模块，覆盖已知历史格式，解析失败时原样输出完整 stderr 而非返回错误哈希。

每个 Derived Hash 需付一次注定失败的 `nix build`，该构建在失败前会完整下载并解析上游依赖，因此必须联网且在 CI 中成本可观。

已在本机 Nix 2.35.2 验证的输出格式：

```
error: hash mismatch in fixed-output derivation '/nix/store/...-README.md.drv':
         specified: sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=
            got:    sha256-5zQ7fQCHY1R4YKoy+HpYDJWBkSPdB8nLP+mkfw0jwV8=
```

## 实测修正：失败的 FOD 不浪费下载

初版本 ADR 声称每个 Derived Hash 需付一次「注定失败且被丢弃」的构建。本机 Nix 2.35.2 实测表明该说法对下载部分不成立：以假哈希构建 `fetchurl` 失败后，正确哈希对应的 store path 已存在。

```
# 假哈希构建，解析出 got: sha256-oGInHZSn4Pimly9qGb5fv7SsQi6Qw6In6TerAGLNPpo=
# 随后 nix eval 求出该哈希的 outPath（eval 不触发构建）
/nix/store/vlx3qzbdl292xbdwmdg08lpaqffv2vli-flake.nix -> IN_STORE
```

因此代价是一次下载而非两次；写回真实哈希后的构建命中 store。该 store path 未被 GC root 保护，GC 后需重新下载。

此结论仅在 `fetchurl` 上验证，尚未在 `buildGoModule` 的 `goModules` FOD 上验证。

另需注意：构建可能在到达哈希比对之前就失败（实测 404 的 `fetchurl` 从未输出 `got:` 行，且退出码为 0）。解析器必须区分「有 got 行」与「更早阶段失败」，不能依据退出码判断。假哈希须使用 SRI 格式，不可使用旧式全零 `sha256`。
