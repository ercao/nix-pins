# 配置文件使用 Nix 而非 TOML

原计划以 TOML 描述 Pin，并用白名单透传影响 Derived Hash 的构建参数（ADR-0004）。改为让用户直接写 Nix：配置是一个接受 `{ pkgs, fake }` 的函数，返回 Pin 属性集，其中 `src` 与 `derive` 由用户用真实的 nixpkgs 函数写出，工具注入假哈希。

```nix
{ pkgs, fake }:
{
  curlie = {
    check.github = "rs/curlie";
    src = pkgs.fetchFromGitHub { owner = "rs"; repo = "curlie"; rev = "v1.8.2"; hash = fake; };
    derive = src: pkgs.buildGoModule {
      pname = "curlie"; version = "1.8.2";
      inherit src;
      vendorHash = fake;
      ldflags = [ "-s" "-w" ];
    };
  };
}
```

工具侧只需 eval 配置并读出各中间 FOD 的 `drvPath`，无需理解任何构建参数的语义。实测该 probe 对 Go 与 npm 两种 builder 同时成立：

```json
{"curlie":{"goModules":"/nix/store/64sjmv5w4m7zl4pwgwjck1d8xnn0f166-curlie-1.8.2-go-modules.drv","npmDeps":null,...},
 "sloc":{"goModules":null,"npmDeps":"/nix/store/2jlf6vk0pv2arhm9js3vk7r8n3yksdy9-sloc-0.3.2-npm-deps.drv",...}}
```

## Considered Options

- TOML 配置：格式稳定、可被非 Nix 工具读取，但每一种需要透传的构建参数都必须在配置 schema 与 Rust 侧显式建模，且白名单永远滞后于 nixpkgs 的实际行为。ADR-0012 中 npm 需要 `postPatch` 引用另一个 `fetchurl` 结果，在 TOML 里无法自然表达。

## Consequences

ADR-0004 的参数白名单机制被取消——用户直接写出真实构建参数，不存在「工具是否透传了它」的问题。

配置文件不再是纯数据，工具无法在不调用 Nix 的情况下列出 Pin。`status` 子命令也需要一次 eval（实测 warm eval 约 0.49s，可接受）。

Pins File（ADR-0002）仍为 JSON，配置与锁文件的格式因此不同：手写的是 Nix，生成的是 JSON。
