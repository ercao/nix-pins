# nix-pins：Rust 实现的 Nix 包版本锁定工具（MVP）

labels: ready-for-agent

## 问题陈述

维护一组 Nix 打包的上游依赖时，现有工具 nvfetcher 带来三类摩擦：

- 它把版本检查委派给 nvchecker，运行时同时需要 Haskell 与 Python 环境。
- 它生成 Nix 表达式（`_sources/generated.nix`），该产物在 code review 中是噪音，在 merge 冲突中难以处理。
- 它不支持 `buildGoModule` 的 `vendorHash`、`buildNpmPackage` 的 `npmDepsHash` 这类必须真实执行一次依赖 vendor 才能得到的哈希；用户只能手工维护它们。

其配置也无法表达真实的构建参数。实测 npm 包 `sloc` 需要在 `postPatch` 里引用另一个单独 fetch 的 lockfile，这在 TOML 配置里无法自然表达。

## 解决方案

一个单静态二进制的 Rust 工具。用户用 Nix 编写配置，直接调用真实的 nixpkgs 函数描述每个 Pin 的 Fetcher 与构建方式；工具注入 Fake Hash，通过 Nix 自身求出全部哈希，写入 JSON 格式的 Pins File；用户通过随工具分发的 Reader 消费该文件。

Derived Hash 与 src hash 走同一条取哈希路径，因此支持一种新的 Derived Hash 只需识别它所属的 Intermediate FOD 名称。

## 用户故事

1. 作为 Nix 打包维护者，我希望只安装一个二进制就能更新依赖版本，以便不必在环境里维护 Haskell 与 Python 运行时。
2. 作为维护者，我希望声明一个 Pin 时只写它的上游位置与构建方式，以便不必把版本号在配置里重复书写。
3. 作为维护者，我希望工具自动查出上游最新版本，以便不必手工浏览 release 页面。
4. 作为维护者，我希望内置 Checker 覆盖 GitHub release、git tag、crates.io、PyPI 与 URL 正则，以便常见上游无需额外配置。
5. 作为维护者，我希望在内置 Checker 不支持某个上游时能用 Escape Hatch 指定外部命令，以便长尾上游不会把我挡死。
6. 作为维护者，我希望 src hash 由 nixpkgs 的真实 Fetcher 产出，以便 `fetchSubmodules`、`leaveDotGit`、`sparseCheckout` 等参数的语义不被近似模拟。
7. 作为维护 Go 包的用户，我希望 `vendorHash` 被自动算出并写入 Pins File，以便不必手工跑一遍构建再复制粘贴哈希。
8. 作为维护 npm 包的用户，我希望 `npmDepsHash` 同样被自动算出，以便 Go 与 npm 包用同一套流程管理。
9. 作为维护 npm 包的用户，我希望在源码树不含 lockfile 时得到一条指向 `postPatch` 提供 lockfile 这一既定做法的提示，以便不必自己从构建日志里推断原因。
10. 作为维护者，我希望配置里能写出任意真实构建参数（`patches`、`proxyVendor`、`postPatch`、`modRoot` 等），以便影响 Derived Hash 的输入不会因工具未透传而算错。
11. 作为维护者，我希望版本未变且构建输入未变时不重算 Derived Hash，以便日常更新不必为每个 Go 包付一次完整依赖下载。
12. 作为维护者，我希望在构建输入变化（例如新增一个修改 `go.mod` 的 patch）时自动重算 Derived Hash，以便不会拿到陈旧且错误的哈希。
13. 作为维护者，我希望 Pins File 是 JSON，以便它的 diff 可读、merge 冲突可解、schema 可迁移。
14. 作为维护者，我希望 Pins File 中未使用的能力整个键缺失而非为 null，以便新增一种能力不会给全部既存条目产生噪音 diff。
15. 作为维护者，我希望通过一个薄 Reader 把 Pins File 转成可用的 `src` 与哈希属性，以便调整 Fetcher 行为时不需要改动工具本身。
16. 作为维护者，我希望单个 Pin 更新失败时其余 Pin 的结果被保留，以便一次上游限流不废弃整轮更新。
17. 作为维护者，我希望更新失败的 Pin 在 Pins File 中保持上一轮的完整条目，以便锁文件始终可构建。
18. 作为在 CI 中运行的用户，我希望存在失败时进程以非零码退出，以便流水线不会静默通过一次不完整的更新。
19. 作为维护者，我希望 `status` 报告各 Pin 的当前版本与上次更新的失败项，以便不联网也能了解锁文件状态。
20. 作为维护者，我希望批量的版本检查并行执行、而昂贵的哈希构建串行执行，以便前者不必白等、后者的日志按包顺序可读。
21. 作为维护者，我希望在开始昂贵的哈希计算前得知需要重算的数量，以便预期本次运行的耗时。
22. 作为维护者，我希望只更新指定的几个 Pin 时直接写它们的名字，以便不必为常见操作构造正则。
23. 作为维护者，我希望哈希解析失败时看到完整的原始构建输出，以便自己诊断而不是面对一句无信息的错误。

## 实现决策

详细决策与实测证据见 `docs/adr/0001`–`0015`。以下为对实现者的摘要。

### 模块与职责

- **CLI 入口**：仅 `update` 与 `status` 两个子命令，位置参数接受 Pin 名字，正则退居 `--filter`（ADR-0009）。
- **Probe**：对用户配置的两阶段求值（ADR-0015）。
- **Checker**：内置 GitHub / git / crates.io / PyPI / URL 正则，外加外部命令 Escape Hatch（ADR-0001）。
- **取哈希**：工具唯一一条取哈希路径，输入是一个 drvPath（ADR-0003、ADR-0010、ADR-0011）。
- **Pins File 读写**：JSON schema 的序列化与反序列化（ADR-0002、ADR-0006）。
- **Reader**：随工具分发的 Nix 表达式，是与 Pins File schema 并列的公共 API（ADR-0002）。

### 配置契约（原型产出）

配置是一个函数，工具注入 `pkgs`、`fake`、`pins` 三个参数。`rev` 引用 `pins.<name>.version` 而非字面量，因此工具永不改写用户的 Nix 文件：

```nix
{ pkgs, fake, pins }:
{
  curlie = {
    check.github = "rs/curlie";
    src = pkgs.fetchFromGitHub {
      owner = "rs"; repo = "curlie";
      rev = pins.curlie.version;
      hash = fake;
    };
    derive = src: pkgs.buildGoModule {
      pname = "curlie"; version = pins.curlie.version;
      inherit src;
      vendorHash = fake;
      ldflags = [ "-s" "-w" ];
    };
  };
}
```

约束：`check` 不得依赖 `pins`。阶段一以空 `pins` 求值配置并只读 `check`，Nix 的惰性保证 `src` 中对 `pins` 的引用不被求值；阶段二注入版本后读取 `src` 与各 Intermediate FOD 的 drvPath（ADR-0015）。

### 取哈希机制

构建对象是承载目标哈希的 Intermediate FOD 而非整个包：`vendorHash` 对应 `goModules`，`npmDepsHash` 对应 `npmDeps`，src hash 对应 `src`（ADR-0011）。构建注定失败，真实哈希从 `got:` 行取回。

退出码不可作为判据：实测一个 404 的 `fetchurl` 以退出码 0 结束且输出中没有 `got:` 行。缺 lockfile 的 `npmDeps` 同样在哈希比对之前失败。因此解析结果必须是「有值」或「无值并附完整原始输出」，不得猜测（ADR-0003、ADR-0012）。

Fake Hash 必须使用 SRI 格式且取固定值——它参与 Intermediate FOD 的 drvPath 计算（ADR-0014）。

### 重算判据

以带 Fake Hash 的 Intermediate FOD 的 drvPath 作为指纹，存入 Pins File；指纹不变则复用既有哈希（ADR-0014）。该 drvPath 由 Nix 依据 derivation 的全部真实输入计算，实测恰好在影响 vendor 的输入变化时变化、在 `ldflags`、`subPackages`、`doCheck` 等不影响的输入变化时保持不变。

因指纹包含 nixpkgs 的贡献，升级 nixpkgs 会触发全部 Derived Hash 重算。这是正确行为，但需在文档中说明其成本。

### Pins File schema

带标签的嵌套结构，未使用的能力整个键缺失（ADR-0006）：

```json
{
  "schemaVersion": 1,
  "pins": {
    "curlie": {
      "version": "v1.8.2",
      "fetcher": { "github": { "owner": "rs", "repo": "curlie", "rev": "v1.8.2" } },
      "hash": "sha256-BlpIDik4hkU4c+KCyAmgUURIN362RDQID/qo6Ojp2Ek=",
      "derived": { "vendorHash": "sha256-GBccl8V87u26dtrGpHR+rKqRBqX6lq1SBwfsPvj/+44=" },
      "fingerprints": { "vendorHash": "/nix/store/...-curlie-1.8.2-go-modules.drv" }
    }
  }
}
```

键序稳定以保证 diff 可读。Pins File 与 Reader 共同构成公共 API，须同步演进。

### 失败与并发语义

失败的 Pin 保持上一轮的完整条目不变，成功的正常写入，退出码区分「全部成功」与「有失败但已写入」（ADR-0007）。stderr 摘要须醒目列出失败项。

Checker 与取哈希使用分离的两级并发：前者默认 8 并发，后者默认串行（ADR-0008）。先完成全部 Checker，再依据指纹算出需要重算的数量并告知用户，然后进入昂贵阶段。

## 测试决策

好的测试只断言外部可观察行为：Pins File 的内容、进程退出码、stderr 中的失败摘要。不断言内部函数调用顺序或中间数据结构。

### Seam

首选 seam 是 CLI 进程边界：在临时目录中放一份配置与（可选的）既存 Pins File，运行构建出的二进制，断言 Pins File 内容与退出码。该 seam 一次覆盖参数解析、两阶段求值、重算判据、失败语义与退出码。

该 seam 需要两个注入点：

1. **`nix` 命令**——通过 PATH 前置一个桩脚本，使其对给定输入返回固定的 eval JSON 与固定的 hash mismatch 输出。这使测试无需联网且可确定地构造失败场景（缺 lockfile、无 `got:` 行、部分失败）。
2. **Checker 的 HTTP 端点**——通过可覆盖的 API base 指向本地桩，使 GitHub Checker 可测。

已有先例：`got:` 行解析与 Pins File 序列化已有单元测试，样本取自本机 Nix 2.35.2 的真实输出，包括缺 lockfile 时的无 `got:` 行样本。这些是 seam 之下的少数例外，因为它们编码的是外部格式契约。

### 覆盖重点

- 首轮运行（无 Pins File）能完成两阶段并产出可构建的 Pins File。
- 指纹未变时不触发哈希构建；构建输入变化时触发。
- 单个 Pin 失败时其条目保持原样、其余条目更新、退出码非零。
- 无 `got:` 行时错误包含完整原始输出。
- 未使用的能力在输出 JSON 中键缺失，且输出不含 null。

## 范围外

- 生成 Nix 表达式形式的产物（ADR-0002 已否决）。
- `add` 子命令与任何对用户配置文件的机器改写（ADR-0009、ADR-0015）。
- nvchecker 的长尾 source（由 Escape Hatch 兜底，ADR-0001）。
- `cargoHash` 等其他 Derived Hash 的内置支持；机制已通用，但 MVP 只验证 `vendorHash` 与 `npmDepsHash`。
- 从 nvfetcher 配置自动迁移。

## 进一步说明

实测基线（本机 Nix 2.35.2）：

```
curlie 1.8.2  src        sha256-BlpIDik4hkU4c+KCyAmgUURIN362RDQID/qo6Ojp2Ek=   8.0s
curlie 1.8.2  goModules  sha256-GBccl8V87u26dtrGpHR+rKqRBqX6lq1SBwfsPvj/+44=   8.3s
sloc   0.3.2  npmDeps    sha256-cFUWwmsYy75qAfhkY6tc4Hxwjo8WJcC5urQC22UnnVU=  12.3s
```

`sloc` 的结果与 nixpkgs 中记录的 `npmDepsHash` 逐字符一致。哈希不匹配而失败的 FOD 仍会把正确哈希对应的输出注册进 store，因此一轮更新中下载只发生一次；该 store path 不受 GC root 保护。

仍未决的实现选择：

- HTTP 客户端与并发运行时的组合（同步客户端配线程池，或异步客户端配 tokio）。ADR-0008 只规定并发度，未规定实现。
- GitHub token 支持。匿名调用实测 rate limit 为 60/小时，对 33 个 Pin 的仓库一轮即耗去约半数配额，因此 token 不是可选项。
- 迁移目标 `macos-config` 的 33 个 Pin 当前无一使用 Derived Hash，其 src 类型只有 `github` 与 `git`。首版落地顺序应先打通版本检查、src hash 与 Reader。
