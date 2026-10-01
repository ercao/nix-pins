# nix-pins

用 Rust 检查 Nix 包的上游版本，计算源码及依赖 Hash，输出可供 Nix 使用的 JSON Pins File。

```sh
cargo run -- status
cargo run -- update
cargo run -- update --config examples/example2/pins-config.nix --pins /tmp/nix-pins.json
```

运行时需要 Nix；默认读取当前目录的 pins-config.nix，写入 pins.json。配置使用 `{ pin }:` 声明，示例见 [examples](examples)。status 只读，update 按 Pin 原子聚合结果，失败保留旧值。

仓库根目录同时管理 Cargo 与 pnpm workspace。Rust CLI 位于 `crates/cli/`，是 Cargo 的默认成员；`nix/` 保存共享 Nix 库，`examples/` 保存 Nix 使用示例，`scripts/` 保存产品验证脚本。文档站位于 [docs](docs/README.md)，内部设计资料位于 `design/`。`Cargo.lock`、`pnpm-lock.yaml` 和 Rust 构建产物目录 `target/` 均保留在根目录。

- [应用配置与并发](docs/content/zh-CN/3.reference/2.configuration.md)：config-rs 优先级，下载默认 2，Checker/Hash 默认 CPU/2。
- [当前项目规格](design/specs/0001-nix-pins-mvp.md)：CLI、配置、持久化和执行契约。
- [多 Source](design/specs/0005-multi-source-pins.md) 与 [pnpm](design/specs/0008-pnpm-dependencies.md)：公开接口与边界。
- [TUI 示例和验证](design/tui-examples.md)：预览全部状态及保存最新截图。
- [领域术语](CONTEXT.md) 与 [当前架构决策](design/adr/README.md)。

```sh
cargo test --all-targets -- --test-threads=1
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

真实网络和依赖构建测试沿用 ignored 标记，显式运行前需准备对应环境。Flake 提供当前 aarch64-darwin 的 package、app、check 和 devShell；package 默认未开启 Cargo tests，构建通过与测试通过分别报告。
