# nix-pins：正交 Source API 与交付加固

status: draft

## 问题

Rust 已实现 `cmd`、GitHub、git、crates.io、PyPI 与 URL Checker，Pins File Reader 已支持 GitHub、git 与 URL Fetcher，但公开 `{ pin }` DSL 只暴露绑定在一起的 `pin.github`。因此底层能力无法通过当前配置 API 正交组合，旧 33 Pin 验证也仍使用 `{ pkgs, fake, pins }` 签名。

Pins File 当前直接覆盖写入；两个并发 Update 可能互相覆盖，进程中断也可能留下损坏文件。CLI 仍固定使用当前目录的配置与 Pins File。

## 目标

1. 补齐已有 Checker 与 Fetcher 的公开声明能力，不增加新 Builder。
2. 保持 `pin.github` 简洁，同时允许 Checker 与 Fetcher 任意正交组合。
3. 保证 Pins File 原子写入并拒绝并发写者。
4. 补齐路径参数与最小 flake 分发。
5. 用新版 DSL 重新完成 33 Pin 真实迁移验证。

## 配置 API

常见 GitHub Pin 保留现有简写：

```nix
curlie = pin.github {
  target = "rs/curlie";
  patches = [./curlie.patch];
  postPatch = "echo patched";
  packages.default = pin.goModule {};
};
```

其他组合使用 `pin.mk`：

```nix
example = pin.mk {
  checker = pin.checker.cmd "git ls-remote ...";
  fetcher = pin.fetcher.git {
    target = "https://example.com/repo.git";
  };
  packages.default = pin.npmPackage {};
};

crate-example = pin.mk {
  checker = pin.checker.crate { target = "demo"; };
  fetcher = pin.fetcher.url {
    target = version: "https://example.com/demo-${version}.tar.gz";
  };
};
```

Builder 未显式设置 `pname` 时，`packages.default` 默认使用 Pin 名，其他具名包默认使用 `${pinName}-${packageName}`；显式 `pname` 始终优先。

`pin.mk` 与 `pin.github` 的 `patches`、`postPatch` 由 `pkgs.applyPatches` 处理。对外 `src` 与全部 Builder 使用处理后的源码；源码哈希仍针对原始 Fetcher 输出计算。

`pin.checker` 暴露：

- `cmd "..."`
- `github { target = "owner/repo"; }`
- `git { target = url; }`
- `crate { target = name; }`
- `pypi { target = name; }`
- `npm { target = name; distTag ? "latest"; }`
- `url { target = url; regex; }`

`pin.fetcher` 暴露：

- `github { target = "owner/repo"; rev ? version: version; fetcherArgs ? {}; }`
- `git { target = url; rev ? version: version; fetcherArgs ? {}; }`
- `url { target = version: ...; fetcherArgs ? {}; }`

公共构造器使用结构化参数。现有 Rust Checker JSON 只作为 Probe 内部契约。未知字段、缺失字段、不可序列化的映射结果，以及 `fetcherArgs` 覆盖 `owner`、`repo`、`url`、`rev`、`hash` 等保留字段，均属于 Configuration Error：在任何 Checker 运行前全局失败且不写 Pins File。

## Version 契约

- Checker 负责枚举和比较候选，只输出一个 Version。
- Version 仅清除首尾空白，不做全局 semver、前缀或大小写规范化。
- Fetcher 映射函数只接收 Version 字符串，只在 source 阶段求值。
- 映射结果进入 Probe JSON；函数本身不得进入 Rust 类型或 Pins File。
- GitHub/git 默认原样使用 Version 作为 `rev`；URL 必须显式提供 Version 到 URL 的函数。
- `cmd` 成功输出必须恰好是一行非空 Version；诊断写 stderr，多行或空输出报错。
- `pins-config.nix` 与 `cmd` 被视为受信任代码，不提供额外沙箱。

## Update 与 Pins File

- 使用 `fs2` 在同目录稳定 sidecar 文件上取得 advisory exclusive lock，从读取 Pins File 前持有到替换完成；锁已占用时立即失败。
- 新内容写入同目录临时文件，完成后 `sync_all` 并原子替换目标；任何失败保留旧文件。
- 新序列化字节与现有文件完全相同时跳过替换，保留 mtime。
- 无 Selection 的完整 Update 删除配置中已不存在的 Pin 与 failure。
- 选择性 Update 不修改未选择条目。
- 新 Pin 首次失败时只记录 failure，不创建不完整 Pin。
- Configuration Error 全局失败；Checker、Fetcher 与哈希计算的运行错误遵循既有 Pin Failure 和部分写入语义。

## CLI

复用 ADR-0018 的 Clap derive 解析与 `src/cli.rs` 边界：

- 新增全局 `--config PATH`，默认 `./pins-config.nix`。
- 新增全局 `--pins PATH`，默认 `./pins.json`。
- 保留默认 Status、`update`、`status`、位置 Pin 名和 `--filter` 并集语义。
- 显式 Pin 名不存在时报错；仅提供 `--filter` 且零匹配时报错。
- `-h`/`--help`、`-V`/`--version` 退出 0；参数错误退出 2。
- Status 不运行 Checker，不增加 `status --refresh`。
- HTTP Checker 保留 30 秒超时；git Checker 设置 `GIT_TERMINAL_PROMPT=0`；不实现统一子进程超时。

## 分发

新增最小 `flake.nix`，初版只声明已验证的 `aarch64-darwin`：

- `packages.default`
- `apps.default`
- `checks.default`

不提供 overlay、NixOS/Home Manager module、devShell、crates.io、预编译二进制、Homebrew 或远程 CI。

## 验证

自动测试继续使用本地桩，覆盖：

- 每个结构化 Checker/Fetcher 构造器；
- 至少一个 Checker 与不同 Fetcher 的交叉组合；
- Version 映射与不可序列化结果；
- 保留字段冲突与 Configuration Error 不写文件；
- `cmd` 空输出、多行输出、stderr 与非零退出；
- 原子替换失败、并发锁、无变化不写；
- 完整 Update 清理与选择性 Update 保留；
- 严格 Selection；
- Clap 路径参数、help、version 与错误退出码；
- flake check。

手工验收使用新版 `{ pin }` DSL 重新迁移 macos-config 的 33 个 Pin，覆盖 GitHub、git、URL Fetcher 与 GitHub、git、cmd Checker。记录命令、耗时、GitHub 配额、33/33 hash 对比、失败日志和第二轮零 hash 重算。该真实网络验证不进入 `cargo test` 或 `flake check`。

## 非目标

- Rust Builder、`cargoLock` 或 `cargoHash`；
- `--dry-run`、独立 `--json` 报告 schema；
- Pins File migration 框架；
- nvfetcher 配置自动迁移；
- `status --refresh`；
- README、shell completion、man page；
- 命令沙箱、白名单或统一子进程超时；
- 新的 Checker、Fetcher、Builder 或 Derived Hash 类型。

## 验收标准

- [ ] 新版 DSL 能表达全部已有 Checker 与 Fetcher，并保留 `pin.github`。
- [ ] Pins File 写入具备进程锁、原子替换和无变化跳过。
- [ ] 完整与选择性 Update、Configuration Error 与 Pin Failure 语义符合本文。
- [ ] CLI 路径、选择器和标准 Clap 行为有自动测试。
- [ ] `aarch64-darwin` 的默认 flake package/app/check 可用。
- [ ] 33 Pin 新版 DSL 手工验证完成并保存证据。
