# 提供 pin.git 便捷构造器

`pin.git` 表示 Git Checker 与 Git Fetcher 的单 Source 便捷组合：默认检查远端 HEAD，并用选出的 commit SHA 获取同一仓库的源码，结果规范化为 `default` Source；多 Source Pin 继续通过 `pin.mk` 表达。它使用扁平参数集，唯一仓库字段为 `target`，并同时供 Checker 与 Fetcher 使用；Checker 专属字段与 Fetcher 专属字段分别传给对应构造器，需要不同 target 的组合仍通过 `pin.mk` 表达。`pin.github` 与 `pin.git` 两个顶层便捷构造器都接受字符串，并将它规范化为 `{ target = value; }`；`pin.checker.*` 与 `pin.fetcher.*` 不增加字符串简写。其他 Checker 与 Fetcher 组合仍通过 `pin.mk` 表达；本决策仅取代 ADR-0017 中不增加 `pin.git` 便捷构造器的部分。
