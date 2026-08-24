# nix-pins

一个 Rust 实现的 Nix 包版本锁定工具（nvfetcher 的替代）。追踪上游版本、预取源码、并计算构建器所需的派生哈希（如 buildGoModule 的 vendorHash），输出可被 Nix 直接消费的锁定结果。

## Language

**Pin**：
配置中一个被追踪的上游包条目，以及它当前锁定的版本与哈希集合。
_Avoid_：package, entry, dependency

**Pin Progress**：
交互式 Update 中以 Pin 为一级展示单元的处理状态，显示当前锁定 Version、Checker 选出的目标 Version，以及当前处理步骤。Checker 完成前目标 Version 可能未知，Fetcher 求值前也不存在可展示的 Source，因此进度不以 Source 为一级分组。
_Avoid_：source progress, download task, stage row

**Version**：
Checker 为 Pin 选出的上游标识；除清除协议性的首尾空白外保留原始文本，并原样记录在 Pins File 中。Fetcher 可以直接使用或映射 Version，但不改变 Pin 锁定的 Version；`v1.2.3` 与 `1.2.3` 是不同 Version。
_Avoid_：rev, tag, release

**Checker**：
判定某个 Pin 上游最新版本的机制（GitHub release、git tags、crates.io 等）。候选版本的枚举与比较属于具体 Checker；Checker 最终只回答“锁定哪个 Version”，不决定如何获取该版本的源码。
_Avoid_：source, nvchecker, version source

**Escape Hatch**：
用户自定义的外部命令形式的 Checker，用于覆盖内置 Checker 不支持的长尾上游。它属于受信任的 `pins-config.nix`，不提供面向不可信配置的沙箱边界。
_Avoid_：custom source, plugin, script

**Fetcher**：
把已确定版本的上游内容取到 store 并得到其哈希的机制。Fetcher 只回答“如何获取源码”，与选择版本的 Checker 正交组合。
_Avoid_：downloader, prefetch backend

**Derived Hash**：
无法从源码 tarball 直接得出、必须通过一次真实构建步骤才能确定的哈希（vendorHash、cargoHash、npmDepsHash）。
_Avoid_：extra hash, secondary hash, build hash

**Pins File**：
工具输出的 JSON 锁文件，记录每个 Pin 的版本、fetcher 参数与全部哈希。
_Avoid_：lockfile, generated.json, manifest

**Configuration Error**：
Pin 声明违反配置契约的确定性错误。它使整次操作在处理任何 Pin 前失败，不产生新的 Pins File。
_Avoid_：pin failure, partial failure

**Pin Failure**：
单个 Pin 在版本检查、取源码或计算哈希时发生的运行失败。其他 Pin 可以继续更新，失败 Pin 保留上一轮可用内容；首次更新且没有旧值时只记录 failure，不创建不可构建的 Pin 条目。
_Avoid_：configuration error, invalid declaration

**Update**：
解析并锁定选中的 Pin，随后写入 Pins File 的命令；它是 CLI 的写操作。未提供 Selection 时，Update 还会删除配置中已不存在的 Pin 与 failure；选择性 Update 不修改范围外条目。
_Avoid_：refresh, sync, generate

**Status**：
只读报告当前 Pin 与上次失败状态的命令；未指定子命令时默认执行 Status。
_Avoid_：check, inspect, list

**Selection**：
一次命令要处理的 Pin 集合；显式名称与 `--filter` 正则取并集，两者都为空时表示全部 Pin。显式名称不存在，或仅提供正则且零匹配，均属于命令错误而非成功空跑。
_Avoid_：scope, targets, matcher

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
