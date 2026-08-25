# Checker、Source Hash 与 Derived Hash 使用分离并发

三类工作的代价不同：Checker 是数百毫秒的 HTTP 请求，Source Hash 主要等待下载，Derived Hash 则可能耗时数分钟并占用网络、CPU 与磁盘。因此不共用默认并发度：Checker 与 Source Hash 默认 8 并发，Derived Hash 默认串行，分别由 `NIX_PINS_CHECKER_JOBS`、`NIX_PINS_DOWNLOAD_JOBS` 与 `NIX_PINS_HASH_JOBS` 控制。

先跑完全部 Checker，再依据 Vendor Inputs Fingerprint 计算真正需要重算的 Derived Hash 列表并告知数量，使用户预期等待时长。

## Considered Options

- 单一并发池（nvfetcher 的 Shake 调度）：调高会让多个 `nix build` 互相抢占网络与磁盘并触发上游 module proxy 限流，调低则让批量 HTTP 版本检查白等。
- 把全部 Derived Hash 构建一次性交给 Nix 调度：不可行。假哈希循环依赖构建失败，多个注定失败的构建其 stderr 会交错，解析器需从混合流中判断每个 `got:` 属于哪个 drv。

## Consequences

三个并发旋钮的解释成本高于一个。Source Hash 默认并发避免多个独立下载串行等待；Derived Hash 串行避免多个昂贵构建互相争抢资源。
