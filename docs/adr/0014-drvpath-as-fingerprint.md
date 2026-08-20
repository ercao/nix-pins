# 以中间 FOD 的 drvPath 作为指纹，取代手工白名单

status: accepted
supersedes: ADR-0005

ADR-0005 计划把影响 Derived Hash 的输入按白名单序列化后哈希，作为重算判据。改为直接使用带假哈希的中间 FOD 的 `drvPath`：它由 Nix 依据该 derivation 的全部真实输入计算，天然精确。

实测（`curlie` 1.8.2 的 `goModules`，假 vendorHash 固定）：

```
baseline                    4vxq82dfdn06r15p75qdws6bpp40rx1s
baseline 重复               4vxq82dfdn06r15p75qdws6bpp40rx1s   稳定
proxyVendor = true          94m59gkfnlzpcc1s8sva36k60kjq95sk   变化
postPatch = "true"          3cg97vmcc47nwdmal5fqwnidlzpfn9k4   变化
modRoot = "./sub"           fhgpnb8x7p8pm0sgjxkv89ydr0ng6h0i   变化
ldflags = [ "-s" "-w" ]     4vxq82dfdn06r15p75qdws6bpp40rx1s   不变
subPackages = [ "." ]       4vxq82dfdn06r15p75qdws6bpp40rx1s   不变
doCheck = false             4vxq82dfdn06r15p75qdws6bpp40rx1s   不变
```

drvPath 恰好在影响 vendor 的输入变化时变化，在不影响的输入变化时保持不变。手工白名单只能逼近这一行为，且实测中 `env` 一项的表现与 `module.nix` 的 `inherit` 列表不一致，说明白名单猜测本就不可靠。

假哈希固定使 drvPath 与 Derived Hash 自身的取值无关（实测：填入真实 vendorHash 后 drvPath 变为 `x7i373p7is6y5207vp2v5mrysal3z81m`，因此指纹必须始终以假哈希求得）。

## Consequences

Pins File 中存储该 drvPath 作为指纹，取代 ADR-0005 的缓存键字段。指纹随 nixpkgs 版本变化而变化，因此升级 nixpkgs 会触发全部 Derived Hash 重算——这是正确行为，但成本高，需在文档中说明。
