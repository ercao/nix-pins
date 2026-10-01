# nix-pins：一个 Pin 锁定多个具名 Source

状态：当前实现契约。

## 问题陈述

同一个上游版本可以对应多个分别获取和构建的源码，例如服务端仓库、前端归档和发布产物。当前模型用一个 Pin 的多个 Source 表达它们的版本归属，并保证整 Pin 的原子性。

## 解决方案

将 Pin 定义为拥有一个 Checker、一个 Version 和一个或多个具名 Source 的原子锁定单元。每个 Source 独立声明 Fetcher、补丁、Package、源码 Hash、Derived Hash 与 Fingerprint，但同一 Pin 的全部 Source 共用 Checker 选出的 Version。

`pin.mk` 新增显式 `sources` 声明。Source 的参数与现有单 Source 顶层参数相同，但不包含 `checker`。为保持简单配置简洁，现有单 Source `pin.mk` 和便捷构造器继续作为 `sources.default` 的简写。存在显式 `sources` 时，以它为准并忽略顶层单 Source 参数。

Pins File 升级为 schema v2：Version 只保存在 Pin 层，Source 数据嵌套在具名 `sources` 下。Update、失败记录、Selection 与持久化均以 Pin 为边界；Source 不能单独更新。Reader 与交互式进度严格保留 Pin → Source → Package 层级。

## 用户故事

1. 作为 Nix 打包维护者，我希望一个 Pin 可以声明多个 Source，以便多个发布产物共用一次上游版本检查。
2. 作为 Nix 打包维护者，我希望每个 Source 有稳定名称，以便区分同一 Version 下的不同仓库、归档或构建输入。
3. 作为 Nix 打包维护者，我希望每个 Source 可以选择不同 Fetcher，以便同一 Version 可以同时映射到 Git、GitHub、URL、zip 或 Hugging Face 来源。
4. 作为 Nix 打包维护者，我希望每个 Source 可以独立映射 Version，以便不同上游来源可以使用不同 rev、标签前缀或 URL 结构。
5. 作为 Nix 打包维护者，我希望每个 Source 拥有自己的 `patches` 与 `postPatch`，以便源码转换只作用于目标 Source。
6. 作为 Nix 打包维护者，我希望每个 Source 拥有自己的 Package 集合，以便 Package 明确使用所属 Source，而不是依赖隐式的 Pin 级源码。
7. 作为现有单 Source 配置的维护者，我希望继续使用顶层 `fetcher`、`patches`、`postPatch` 与 `packages`，以便简单配置不必增加嵌套层级。
8. 作为配置作者，我希望单 Source 顶层参数被规范化为名为 `default` 的 Source，以便内部始终只有一种数据模型。
9. 作为配置作者，我希望显式 `sources` 存在时优先使用它，以便可以逐步加入多 Source 声明，而不会因遗留顶层参数产生冲突错误。
10. 作为配置作者，我希望 Source 声明不能包含 Checker，以便一个 Pin 始终只有一个 Version 决策来源。
11. 作为配置作者，我希望空的 `sources` 或缺少 Fetcher 的 Source 在处理开始前成为 Configuration Error，以便不会写入不可构建的 Pins File。
12. 作为用户，我希望 `update` 对每个 Pin 只运行一次 Checker，以便共享版本检查不会产生重复网络请求或命令副作用。
13. 作为用户，我希望 Pin 的所有 Source 使用同一个 Target Version，以便它们始终代表同一次上游发布。
14. 作为用户，我希望 Pin 的所有 Source 原子提交，以便任一 Source 失败时不会留下混合 Version。
15. 作为用户，我希望 Source 失败时整 Pin 保留上一轮可用结果，以便当前构建继续可用。
16. 作为首次锁定 Pin 的用户，我希望任一 Source 失败时不创建不完整 Pin，只记录失败，以便 Reader 不会暴露不可构建的数据。
17. 作为用户，我希望一个 Pin Failure 只记录一次，并指出实际失败的 Source 与 Hash 步骤，以便失败摘要准确且不重复。
18. 作为 CLI 用户，我希望只能选择 Pin 而不能选择 Source，以便 Selection 与原子 Update 边界一致。
19. 作为 CLI 用户，我希望 `--filter` 继续匹配 Pin 名，以便多 Source 不改变现有选择模型。
20. 作为并行更新用户，我希望不同 Pin 的 Source 以及同一 Pin 的不同 Source 可以共享现有 Hash 并发池，以便提高吞吐量而不增加新的配置项。
21. 作为并行更新用户，我希望并行完成的 Source 结果在 Pin 层聚合后再提交，以便并发不破坏原子性。
22. 作为 Pins File 阅读者，我希望 Version 只在 Pin 层出现一次，以便锁文件准确表达版本所有权。
23. 作为 Pins File 阅读者，我希望每个 Source 分别保存 Fetcher、源码 Hash、Derived Hash 与 Fingerprint，以便缓存判定和构建输入保持独立。
24. 作为 Nix 消费者，我希望 Reader 暴露 `Pin → Sources → Source` 层级，以便通过稳定路径取得每个 Source 的 `src`、Derived Hash 与 Package。
25. 作为 Nix 消费者，我希望 Reader 不为唯一 `default` Source 提升属性，以便单 Source 与多 Source 的消费模型完全一致。
26. 作为终端用户，我希望进度以 Pin 为一级、Source 为二级、Package 为三级，以便能定位当前处理的 Source 与 Package。
27. 作为简单 Pin 的用户，我希望唯一 `default` Source 和唯一 `default` Package 可以折叠，以便常见场景保持紧凑。
28. 作为故障排查者，我希望 Source Progress 显示 Fetcher、源码 Hash、补丁和 Nix 详情，以便区分 Source 与 Derived Hash 阶段。
29. 作为故障排查者，我希望 Package Progress 只显示 Derived Hash 工作，以便职责层级清晰。
30. 作为仓库维护者，我希望 schema v1 不再限制新模型，以便实现可以直接采用正确的嵌套结构而无需兼容分支。

## 实现决策

- Pin 是 Checker、Version、Selection、Update、失败记录与原子提交的所有者。
- Source 是 Pin 下的具名源码获取单元；Source 拥有 Fetcher、补丁、Package、源码 Hash、Derived Hash 与 Fingerprint。
- Package 属于 Source，并共享所属 Pin 的 Version 与所属 Source 的源码。
- 公共声明继续使用 `pin.mk`。多 Source 使用 `sources.<name>`；不新增 `pin.group`、Shared Checker 或 Checker Group 抽象。
- Source 参数与现有单 Source 顶层参数相同，但排除 `checker`。
- 存在显式 `sources` 时，只使用其中的 Source 声明，并忽略顶层 `fetcher`、`patches`、`postPatch` 与 `packages`。
- 不存在显式 `sources` 时，将现有顶层单 Source 参数规范化为 `sources.default`。
- 现有单 Source 便捷构造器继续生成 `default` Source。
- 显式 `sources` 必须是非空属性集；每个 Source 必须包含 Fetcher。无效声明属于 Configuration Error，必须发生在任何 Checker 执行前。
- Checker 对每个选中 Pin 只执行一次，选出的 Target Version 分发给全部 Source。
- Pins File 使用 schema v2。每个 Pin 保存一次 Version，并在 `sources` 下按 Source 名保存 Fetcher、源码 Hash、Derived Hash 与 Fingerprint。
- 不读取、迁移或兼容 schema v1 Pins File；schema 不匹配时返回明确错误。
- Reader 严格暴露 Pin → Sources → Source 层级。Source 下分别暴露 `src`、`derived` 与 `packages`，不提升唯一 Source 或 Package 的属性。
- Update 不能选择单个 Source；Selection 继续只接受 Pin 名，`--filter` 继续匹配 Pin 名。
- Pin 的更新结果必须原子提交。任一 Source 的源码 Hash、补丁求值或 Derived Hash 失败时，全部新 Source 结果丢弃，旧 Pin 保持不变。
- Pin Failure 只以 Pin 名记录一次；错误文本包含实际失败 Source、Package 和 Hash 步骤。多个失败按稳定 Source/Package 顺序汇总。
- Source 下载使用 Download 队列，Nix 求值与 Derived Hash 使用 Hash 队列；并发配置统一由 config-rs 加载，见 [应用配置](../../docs/content/zh-CN/3.reference/2.configuration.md)。
- 并行任务可以继续完成并收集错误，但只有 Pin 的全部任务成功后才能生成新 Pin。
- 进度模型为 Pin → Source → Package。Pin 行显示 Checker、Current Version、Target Version 与整体提交状态；Source 行显示 Fetcher、Source Hash、补丁与 Nix 详情；Package 行显示 Derived Hash。
- 唯一名为 `default` 的 Source 可以折叠到 Pin 行；唯一名为 `default` 的 Package 可以折叠到所属 Source 行。折叠只影响显示，不改变数据模型。
- `Processed`、失败数量和退出码继续按 Pin 计算，不按 Source 或 Package 计算。
- 既有正交 Checker/Fetcher、两阶段 Probe、Fake Hash、Fingerprint 复用、Pins File 事务写入与部分 Pin 失败语义继续使用；本功能只把单 Pin 内的 Source 数量从一个扩展为多个。

## 测试决策

- 使用两个公开行为 seam，不以内部函数调用作为主要验收依据。
- 第一 seam 是端到端 CLI 集成：运行编译后的 `update` 与 `status`，从声明式配置经过 Checker、Probe、Hash 计算、事务写入直到生成 schema v2 Pins File。
- CLI seam 必须证明一个多 Source Pin 的 Checker 只执行一次，且全部 Source 获得同一个 Version。
- CLI seam 必须验证 Git/URL 等不同 Fetcher 可以存在于同一 Pin，并分别写入嵌套 Source 数据。
- CLI seam 必须验证现有单 Source `pin.mk` 与便捷构造器被规范化为 `sources.default`。
- CLI seam 必须验证显式 `sources` 优先于遗留顶层 Source 参数。
- CLI seam 必须验证空 Sources、Source 内 Checker、缺少 Fetcher 等错误在 Checker 执行前失败，并且 Pins File 字节不变。
- CLI seam 必须验证任一 Source 或 Package Hash 失败时整 Pin 回滚；其他独立 Pin 仍可按既有部分失败语义成功提交。
- CLI seam 必须验证失败只记录在 Pin 上一次，且错误包含 Source/Package 定位。
- CLI seam 必须验证首次更新失败不产生部分 Pin。
- CLI seam 必须验证 Source 名不能作为 Selection，Pin 名和 Pin 级 `--filter` 仍有效。
- CLI seam 必须在生成 Pins File 后调用公共 Nix Reader，形成从生产到消费的完整端到端检查。
- 第二 seam 是公共 Nix Reader 求值：直接消费 schema v2 Pins File，验证 Version、Source src、Derived Hash 与 Package 的嵌套访问结构。
- Reader seam 必须覆盖多个 Source、单个 `default` Source、多个 Package，以及 Go/npm Intermediate FOD 的归属。
- Reader seam 必须证明没有单 Source 或单 Package 属性提升，并对 schema v1 返回明确的不支持错误。
- 进度渲染可以沿用现有内存终端测试作为补充，验证多 Source 树、默认节点折叠、失败后折叠和静态输出；这些测试不替代 CLI seam。
- Pins File 序列化单元测试可以补充验证稳定键序、空能力省略、schema v2 roundtrip 与事务写入；业务原子性仍由 CLI seam 验证。
- 测试应断言用户可观察行为：JSON 形状、Reader 求值结果、Checker 调用次数、Pins File 是否变更、退出码和稳定失败定位；不测试线程调度顺序或内部任务类型。

## 范围外

- schema v1 Pins File 的读取、自动迁移或兼容输出。
- 多个 Pin 自动按 Checker 内容去重。
- Shared Checker、Checker Group 或独立的 `pin.group` DSL。
- Source 级 Selection、Update、重试或单独提交。
- Source 专用并发环境变量或 CLI 参数。
- 为唯一 `default` Source 或 Package 提升 Reader 属性。
- 修改 Checker 的候选版本选择、重试或 Version 文本语义。
- 新增 Fetcher、Builder 或 Derived Hash 类型。
- 跨 Pin 原子提交；既有独立 Pin 之间仍保持部分失败语义。
- 远程 Issue、迁移工具或 schema v1 转换脚本。

## 进一步说明

- 术语以项目领域词汇表中的 Pin、Source、Package、Checker、Fetcher、Version、Pin Failure、Selection 与 Progress 定义为准。
- Pin 内原子性与 Pin 间部分失败并存：一个多 Source Pin 要么整体采用新 Version，要么整体保留旧结果；其他 Pin 不因该 Pin 失败而回滚。
- 显式 `sources` 覆盖顶层单 Source 参数是有意的优先级规则，不是兼容性警告或配置错误。
- 规范完成后，应以对应 ADR 为架构依据；若实现发现必须改变 Pin、Source 或原子提交边界，应先更新领域文档与 ADR，再修改代码。
