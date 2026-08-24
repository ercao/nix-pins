# nix-pins：Checker/Fetcher 能力扩展

status: ready
labels: ready-for-agent

## 问题陈述

nix-pins 已经具备多种 Checker、三种 Pins File Fetcher，以及两阶段 Probe、源码哈希和派生哈希锁定能力，但公开 Nix DSL 仍主要通过 `pin.github` 将 GitHub Checker 与 GitHub Fetcher 绑定在一起。用户无法在声明层独立组合“如何发现版本”和“如何取得源码”，也无法声明 Git 或 URL Fetcher。

现有 Git Checker 只选择远端 tag，不能锁定 HEAD、分支或指定 ref；tag 也缺少过滤和显式排序策略。远程 HTTP Checker 遇到临时网络错误、限流或服务端错误时立即失败，npm registry 则没有内置 Checker。这些缺口使 nix-pins 难以覆盖 nvfetcher 常见的非 Rust 更新场景。

## 解决方案

在现有两阶段公开 Nix seam 上暴露正交的 Checker 与 Fetcher 构造器，并保留 `pin.github` 作为兼容便捷写法。新增声明式 Git 与 URL Fetcher，使任意 Checker 返回的 Version 可以通过原生 Nix 函数映射为 Fetcher 参数。

扩展 Git Checker，使其支持 tag、HEAD、分支和指定 ref，并为 tag 模式提供 include、exclude 与排序选项。为所有幂等 HTTP Checker 增加共用的有界重试行为，并新增支持 npm dist-tag 的 npm Checker。

## 用户故事

1. 作为 nix-pins 用户，我希望独立选择 Checker 与 Fetcher，以便按上游发布方式和源码托管方式自由组合。
2. 作为现有配置的维护者，我希望 `pin.github` 保持兼容，以便升级后不必立即迁移已有配置。
3. 作为配置作者，我希望使用统一的公开构造器声明 Pin，以便不依赖内部 Probe JSON 契约。
4. 作为 Git 仓库用户，我希望用任意 Checker 的 Version 选择 Git rev，以便锁定不托管在 GitHub Release 上的源码。
5. 作为 Git Fetcher 用户，我希望用原生 Nix 函数转换 Version，以便添加 tag 前缀或选择不同的 ref 格式。
6. 作为 URL 发布物用户，我希望根据 Version 生成下载 URL，以便锁定 tarball、zip 或其他固定产物。
7. 作为 URL Fetcher 用户，我希望能传递合法的底层 Fetcher 参数，以便使用镜像、名称或解包相关选项。
8. 作为配置维护者，我希望保留字段不能被通用参数覆盖，以便 Pins File 中的 rev、url 和 hash 始终来自明确的锁定流程。
9. 作为追踪开发分支的用户，我希望锁定远端 HEAD 当前指向的 commit，以便每次 Update 都得到可复现的提交 SHA。
10. 作为追踪指定分支的用户，我希望按分支名解析当前 commit，以便不依赖 tag 发布。
11. 作为追踪任意 Git ref 的用户，我希望按完整 ref 解析当前 commit，以便支持项目自定义的 refs 布局。
12. 作为依赖 tag 的用户，我希望 tag 模式保持默认行为，以便现有 Git Checker 配置继续工作。
13. 作为 tag 命名不规则项目的用户，我希望先包含匹配的 tag，再排除不需要的 tag，以便去掉预发布版、平台专用版或无关命名空间。
14. 作为版本排序规则特殊的用户，我希望显式选择 semver 或字典序，以便候选选择符合上游约定。
15. 作为配置作者，我希望最终写入 Pins File 的 Version 保留上游原始字符串，以便 Fetcher 映射和状态输出不丢失信息。
16. 作为使用公共 registry 的用户，我希望临时网络错误能够自动重试，以便一次短暂抖动不会导致 Pin 更新失败。
17. 作为受到 API 限流的用户，我希望客户端遵守 `Retry-After`，以便减少无效请求并更可靠地恢复。
18. 作为排查配置问题的用户，我希望永久性 HTTP 错误和响应解析错误立即失败，以便错误不会被无意义地重复请求掩盖。
19. 作为 npm 包用户，我希望从指定 dist-tag 获取版本，以便锁定 `latest` 或项目自定义发布通道。
20. 作为 scoped npm 包用户，我希望包名被正确编码，以便 `@scope/name` 与普通包行为一致。
21. 作为批量更新用户，我希望单个 Checker 或 Fetcher 的运行失败继续遵循已有 Pin Failure 与部分写入语义，以便其他 Pin 仍可完成更新。
22. 作为配置错误的用户，我希望错误在任何 Checker 运行前被报告且 Pins File 不被写入，以便无效声明不会产生部分状态。

## 实现决策

- Checker 与 Fetcher 是独立概念：Checker 只决定 Version，Fetcher 只决定如何获取该 Version 对应的源码。
- 公开 DSL 提供 Checker 构造器、Fetcher 构造器和组合 Pin 的入口；现有 `pin.github` 等价于 GitHub Checker 与 GitHub Fetcher 的便捷组合。
- 继续使用现有两阶段 Probe：第一阶段取得 Checker 声明，第二阶段在已知 Version 后解析 Fetcher、源码和派生构建。
- Checker 输出只清除协议性的首尾空白。Version 不做全局前缀移除、大小写转换或 semver 规范化。
- GitHub 与 Git Fetcher 默认将 Version 原样映射为 rev，并允许配置通过原生 Nix 函数覆盖映射。
- URL Fetcher 必须提供从 Version 到 URL 的原生 Nix 函数；静态 URL 不作为隐式默认，因为它不能表达版本更新。
- Git、URL 与 GitHub Fetcher 最终继续使用现有 Pins File Fetcher schema，不增加新的锁文件 schema 版本。
- 通用 Fetcher 参数不得覆盖由锁定流程拥有的标识字段和完整性字段，包括 owner、repo、url、rev 与 hash。冲突属于 Configuration Error。
- Git Checker 的默认模式为 HEAD。HEAD、branch 和 ref 模式都将远端对象解析为 commit SHA，并以该 SHA 作为 Version；tag 模式需显式声明。
- branch 模式要求分支名，ref 模式要求完整 ref；缺失或互相冲突的参数属于 Configuration Error。
- tag include 与 exclude 使用正则表达式匹配原始 tag。先执行 include，再执行 exclude；过滤后没有候选属于该 Pin 的 Checker 运行失败。
- tag 排序支持 `semver` 与 `lexicographic`。`semver` 保持现有兼容排序规则，`lexicographic` 直接比较原始 tag；被选中的原始 tag 写入 Pins File。
- HTTP 重试集中在共用的幂等 HTTP GET seam，不为每个 Checker 分别实现。
- 仅对连接类临时错误、HTTP 429、500、502、503 和 504 重试；其他 4xx、成功响应的解析错误和业务字段缺失不重试。
- 每次请求最多执行三次额外重试。无服务端提示时依次等待 250ms、500ms 和 1s；`Retry-After` 的正整数秒形式优先，但单次等待最多 30s。
- 重试适用于 GitHub、crates.io、PyPI、URL regex 和 npm Checker；外部命令与 Git 命令不走 HTTP 重试。
- npm Checker 从 npm registry 元数据的 `dist-tags` 读取指定 tag；dist-tag 默认是 `latest`，返回值仍是原始版本字符串。
- npm registry 基址继续通过 Checker 运行选项注入，以便测试使用本地 HTTP 服务，不新增通用插件接口。
- Configuration Error 在任何 Checker 执行前全局失败且不写 Pins File；远程请求、Git 解析和无候选等运行错误继续使用现有 Pin Failure 与部分写入语义。

## 测试决策

- 主要测试 seam 是公开 Nix DSL 到两阶段 Probe、Update 和 Pins File 的完整外部行为；测试不直接断言 Nix 模块内部属性布局。
- 正交 API、Git Fetcher 与 URL Fetcher 各至少有一个通过公开配置入口的端到端测试，证明 Checker、Version 映射、源码 hash 和 Pins File Fetcher 一致。
- `pin.github` 使用现有公共 Nix seam 的兼容测试，证明迁移前后的 Checker、Fetcher 与包声明行为不变。
- Git Checker 使用临时本地 Git 仓库测试 tag、HEAD、branch 和完整 ref，避免依赖真实网络服务。
- tag 过滤和排序直接测试候选集合的外部选择结果，覆盖 include、exclude、无候选、semver、字典序和原始 Version 保留。
- HTTP 重试使用标准库本地 HTTP 服务，覆盖立即成功、临时失败后成功、429 与 `Retry-After`、达到重试上限、不可重试 4xx 和解析错误。
- npm Checker 使用本地 registry 响应测试普通包、scoped 包、默认 `latest`、自定义 dist-tag 和缺失 dist-tag。
- Configuration Error 测试证明 Checker 未被调用且 Pins File 保持字节级不变；Pin Failure 测试证明其他 Pin 仍能成功写入。
- 复用现有 Pins File roundtrip、Probe、CLI update 和选择性 Update 测试先例，不新建第二套执行框架或集成测试 harness。
- 完成后运行 Rust 测试、Clippy warnings-as-errors、格式检查和 diff hygiene；真实公共 registry 网络访问不进入默认测试套件。

## 范围外

- Rust Builder、Cargo.lock 提取、cargoHash、outputHashes 或任何 Rust 包构建能力。
- Checker 结果缓存、TTL、持久缓存 schema 或离线模式。
- GitLab、Gitea、AUR、Repology、Marketplace、Docker、OCI 等新增后端。
- 通用 Checker/Fetcher 插件系统、动态加载或 trait 抽象。
- Git commit、changelog、PR 或发布自动化。
- GitHub prerelease 策略、GitHub GraphQL 或新的认证体系。
- 原子写入、并发锁、CLI 路径参数、JSON 报告、flake 分发或既有配置的批量迁移。
- 修改 Pins File schema 版本或改变既有部分失败语义。

## 进一步说明

本规范选择补齐当前实际存在的组合缺口，而不是逐项复制 nvfetcher。后续只有在真实 Pins 需要时，再独立评估 GitLab、Manual Checker、GitHub prerelease 或更多版本源。

`Retry-After` 本阶段只承诺正整数秒形式；若实际服务依赖 HTTP-date 形式，再单独扩展解析能力。
