# nix-pins

一个 Rust 实现的 Nix 包版本锁定工具（nvfetcher 的替代）。追踪上游版本、预取源码、并计算构建器所需的派生哈希（如 buildGoModule 的 vendorHash），输出可被 Nix 直接消费的锁定结果。

## Language

**Pin**：
配置中一个被追踪的上游包条目，以及它当前锁定的版本与哈希集合。
_Avoid_：package, entry, dependency

**Checker**：
判定某个 Pin 上游最新版本的机制（GitHub release、git tags、crates.io 等）。
_Avoid_：source, nvchecker, version source

**Escape Hatch**：
用户自定义的外部命令形式的 Checker，用于覆盖内置 Checker 不支持的长尾上游。
_Avoid_：custom source, plugin, script

**Fetcher**：
把已确定版本的上游内容取到 store 并得到其哈希的机制。
_Avoid_：downloader, prefetch backend

**Derived Hash**：
无法从源码 tarball 直接得出、必须通过一次真实构建步骤才能确定的哈希（vendorHash、cargoHash、npmDepsHash）。
_Avoid_：extra hash, secondary hash, build hash

**Pins File**：
工具输出的 JSON 锁文件，记录每个 Pin 的版本、fetcher 参数与全部哈希。
_Avoid_：lockfile, generated.json, manifest

**Reader**：
随工具分发的薄 Nix 表达式，把 Pins File 转换为可用的 `src` 与哈希属性。
_Avoid_：generated.nix, bridge, shim

**Vendor Inputs Fingerprint**：
影响某个 Derived Hash 的全部输入（src hash 与白名单构建参数）的哈希，用于判定该 Derived Hash 是否需要重算。
_Avoid_：cache key, hash of hashes, revision key

**Intermediate FOD**：
承载某个 Derived Hash 的中间 fixed-output derivation attribute，如 buildGoModule 的 `goModules`、buildNpmPackage 的 `npmDeps`。假哈希循环构建它而非整个包。
_Avoid_：vendor derivation, deps drv, sub-derivation

**Fake Hash**：
工具注入配置的固定 SRI 占位哈希。以它求值可得到中间 FOD 的稳定 drvPath；以它构建则从失败输出取回真实哈希。
_Avoid_：dummy hash, placeholder, lib.fakeHash

**Probe**：
工具对用户 Nix 配置的一次求值，用于列出各 Pin 及其中间 FOD 的 drvPath，不触发构建。
_Avoid_：eval pass, discovery, introspection
