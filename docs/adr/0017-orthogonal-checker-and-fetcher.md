# Checker 与 Fetcher 在 Pin 声明中正交

status: accepted

Pin 声明分别选择 Checker 与 Fetcher：Checker 决定锁定哪个版本，Fetcher 决定如何取得该版本的源码。保留 `pin.github` 作为 GitHub Checker 与 GitHub Fetcher 的便捷写法，但它只是正交声明的简写，不把两者语义绑定。

Checker 只产出一个 Version 字符串，该值原样写入 Pins File。Fetcher 默认直接使用 Version；需要拼接 URL、添加前缀或选择其他 ref 时，通过原生 Nix 函数把 Version 映射为 Fetcher 参数，不引入模板语言或结构化 Checker 输出。

Version 保留 Checker 返回的上游标识，只清除协议性的首尾空白，不进行全局 semver 规范化、前缀移除或大小写转换。因此 `v1.2.3` 与 `1.2.3` 是不同 Version；Fetcher 所需的前缀或 ref 转换由映射函数完成。

候选版本的枚举和比较完全属于具体 Checker。git Checker 可以用 semver 选择 tag，其他 Checker 可以使用日期、API 顺序或自身协议；nix-pins 不提供跨 Checker 的全局 Version 排序器。Checker 输出仍是被选中候选的原始 Version。

真实 Pin 不总是按来源一一对应：外部命令可以检查版本后交给 git 或 URL Fetcher，crates.io 可以提供版本而源码由 URL Fetcher 获取。继续增加隐式绑定的 `pin.github`、`pin.git`、`pin.url` 会遗漏这些交叉组合，并使 Escape Hatch 无法与内置 Fetcher 复用。

Pins File 仍只保存已解析的版本与 Fetcher 数据，schema 不因 Checker 声明方式改变。

显式组合使用一个普通构造器，不依赖 `__functor`：

```nix
pin.mk {
  checker = pin.checker.cmd "...";
  fetcher = pin.fetcher.url {
    url = version: "https://example.com/pkg-${version}.tar.gz";
  };
  packages.default = pin.goModule {};
}
```

## Considered Options

- 按来源提供绑定 Checker 与 Fetcher 的构造器：常见场景最短，但交叉组合需要不断增加特例。
- 让用户直接填写原始 Checker 与 Fetcher attrset：组合能力完整，但泄漏内部求值和序列化契约。
- 使用正交的 Checker、Fetcher 声明并为常见组合提供简写：保留组合能力，同时允许简单配置保持简洁。

## Consequences

配置 API 必须能独立表达 Checker 与 Fetcher，并测试 GitHub、git、URL、crates.io 与外部命令之间的交叉组合。

现有 `pin.github` 保留，并明确展开为 GitHub Checker 加 GitHub Fetcher。其他组合通过 `pin.mk` 声明，Checker 与 Fetcher 构造器分别位于 `pin.checker` 和 `pin.fetcher` 命名空间。

首轮公共 API 补齐全部已有后端：`pin.checker` 暴露 `cmd`、`github`、`git`、`crate`、`pypi`、`url`，`pin.fetcher` 暴露 `github`、`git`、`url`。本决策不增加新的 Builder 或 Derived Hash 类型。

公共 Checker 与 Fetcher 构造器使用结构化参数，例如 GitHub Checker 接收 `owner` 与 `repo`，git Checker 接收 `url`。`cmd` Checker 仍直接接收命令字符串。Rust 反序列化使用的紧凑字符串或 tagged JSON 形状只属于内部 Probe 契约，不作为用户配置 API。

`cmd` Checker 成功时 stdout 必须在清除首尾空白后恰好包含一行非空 Version；空输出或多行输出均报错。诊断信息写入 stderr，非零退出时 stderr 作为失败原因保留。

`pins-config.nix` 与其中的 `cmd` Checker 被视为受信任代码。命令继续通过 shell 执行，不增加命令白名单、权限降级或额外沙箱；命令执行失败按 Pin Failure 处理。

结构化构造器在 Nix 求值阶段报告缺失或未知字段，并在交给 Rust 前转换成既有 Checker JSON 形状。

字段缺失、未知字段、`fetcherArgs` 保留字段冲突以及不可序列化的映射结果属于 Configuration Error。它们在运行任何 Checker 前使整次操作失败，且不得写入 Pins File。部分失败语义只用于 Checker、Fetcher 或哈希计算期间的单 Pin 运行失败。

Fetcher 构造器保留 `fetcherArgs` 逃生口，用于 `fetchSubmodules` 等底层 Nix fetcher 的长尾参数。工具管理的标识、版本和完整性字段不可由 `fetcherArgs` 覆盖，包括 `owner`、`repo`、`url`、`rev` 与 `hash`；发生冲突时在 Nix 求值阶段报错，而不是静默决定合并优先级。

Probe 的 checker 阶段只消费 Checker；source 阶段只消费 Fetcher。Builder、Derived Hash 与 Pins File schema 不受此决策影响。

Fetcher 参数中的 Version 映射函数只在已有锁定 Version 时求值。版本检查、状态展示和变更判断均使用 Checker 产出的原始 Version，而非 Fetcher 映射后的 rev 或 URL。

映射函数只存在于 Nix 配置求值期间：checker 阶段不强制求值，source 阶段取得 Version 后求值，其结果必须是可序列化的 Fetcher 参数。函数本身不得进入 Probe JSON、Rust 类型或 Pins File。

映射函数只接收 Version 字符串。Pin 名称、上一轮锁定值或其他运行时上下文不属于调用协议；配置需要的静态信息由原生 Nix 闭包捕获。

GitHub 与 git Fetcher 默认使用 `version: version` 作为 `rev` 映射，并允许显式覆盖该函数。URL Fetcher 没有可靠通用默认，必须提供 `url = version: ...`，不接受静态 URL 字符串。
