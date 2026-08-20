# Checker 与 Derived Hash 使用分离的两级并发

两类工作的代价相差三个数量级：Checker 是数百毫秒的 HTTP 请求，Derived Hash 是可能耗时数分钟的 `nix build`，且后者自身已在打满网络与磁盘并受 Nix daemon `max-jobs` 约束。因此不共用并发池：Checker 默认 8 并发（可调），Derived Hash 默认串行，由独立开关控制。

先跑完全部 Checker，再依据 Vendor Inputs Fingerprint 计算真正需要重算的 Derived Hash 列表并告知数量，使用户预期等待时长。

## Considered Options

- 单一并发池（nvfetcher 的 Shake 调度）：调高会让多个 `nix build` 互相抢占网络与磁盘并触发上游 module proxy 限流，调低则让批量 HTTP 版本检查白等。
- 把全部 Derived Hash 构建一次性交给 Nix 调度：不可行。假哈希循环依赖构建失败，多个注定失败的构建其 stderr 会交错，解析器需从混合流中判断每个 `got:` 属于哪个 drv。

## Consequences

两个并发旋钮的解释成本高于一个。Derived Hash 串行使日志按包顺序可读，失败归属明确。
