# 用最小合成表达式计算 Derived Hash，透传影响 vendor 的参数

计算 Derived Hash 需要一个可构建的 derivation。本工具不引用用户自己的包表达式，而是从 Pin 的版本与 src 合成一个最小的 `buildGoModule` 调用，并注入假哈希。这样配置格式不与用户的包组织方式（flake attr、相对路径、npins）耦合，也无需在 `override` 与 `overrideAttrs` 之间猜测。

## Considered Options

- 指向用户真实的包表达式并 override src 与假哈希：能自动获得全部真实构建参数，但需支持多种引用方式，且 override 机制的选择取决于用户如何编写包。

## Consequences

最小合成表达式并非在所有情况下都正确。`pkgs/build-support/go/module.nix` 中 `goModules` FOD 显式继承了下列属性，任一存在都会改变 vendorHash：

```
src  modRoot  goSum  proxyVendor
prePatch  patches  patchFlags  postPatch
preBuild  modPostBuild
sourceRoot  setSourceRoot  env
```

因此配置需以白名单方式支持透传这些参数，而不是开放任意 Nix 片段。特别注意 `patches` 会影响 vendorHash——打补丁修改 `go.mod` 的包若未透传 patches 将得到错误哈希。

## 被 ADR-0013 取代

status: superseded by ADR-0013

配置改为 Nix 后，用户直接写出真实构建参数，白名单透传机制不再需要。本 ADR 中记录的 `goModules` FOD 继承属性列表仍有参考价值（说明哪些输入会影响 vendorHash），但不再作为工具需要实现的机制。
