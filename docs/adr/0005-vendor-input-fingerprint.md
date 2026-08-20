# 以 vendor 输入指纹决定是否重算 Derived Hash

Derived Hash 的重算成本高（一次完整的上游依赖下载），必须可缓存。判定条件不使用「版本变化」或「src hash 变化」，而是把 ADR-0004 白名单中的全部 vendor 输入参数与 src hash 一起序列化并哈希，作为指纹存入 Pins File；指纹不变即复用既有哈希。

## Considered Options

- 版本变化即重算：版本未变但用户新增 patch 时会保留陈旧哈希，属静默错误。
- src hash 变化即重算：能捕获 tag 被移动，但同样漏掉 patches 等非 src 输入的变化。

## Consequences

Pins File 中出现一个缓存键字段，属实现细节进入公共 schema。收益是「哪些输入影响该哈希」被显式编码，且锁文件 diff 可自解释重算原因。

## 被 ADR-0014 取代

status: superseded by ADR-0014
