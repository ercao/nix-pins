# 按职责整理应用模块，保留现有执行与配置契约

main.rs 只解析入口并调用 app::run。cli.rs 处理 Clap 与配置来源，settings.rs 保留唯一的 Settings 声明；selection.rs 管理 Pin 名称与正则的并集、全量选择和名称验证。应用执行接收项目自己的 Invocation 与 Selection，不依赖 Clap 类型。

app/mod.rs 分派命令并保留共同错误处理，status.rs 负责只读状态，update.rs 负责更新事务、阶段与结果汇总，pipeline.rs 负责 Checker 与各 Pin 的源码和派生哈希调度。checker.rs、probe.rs、pins.rs 继续保留当前组织，不按每种 Checker 或任务类型继续拆文件。

process.rs 统一管理取消信号、子进程组和输出回收，Checker 与 Nix 执行直接依赖它。nix/mod.rs 保留执行和哈希结果判断，nix/log.rs 负责日志解析与文本净化；Nix 不依赖进度展示。progress/mod.rs 管理渲染生命周期，state.rs 保留状态树与 Reporter，plain.rs 负责文本和静态树输出，tui.rs 负责动态视图与终端恢复。内部类型仅向需要它们的父模块开放。

本次只调整职责与依赖方向，不引入 trait、插件、宏或依赖。保留全部参数和四个 API base、CLI > 环境变量 > TOML > 默认值的优先级、原子保存、取消退出 130，以及 plain/TUI 输出。新增文件使职责更容易定位，代价是进度子模块仍共享内部状态类型；不为拆文件额外建立公共接口。

完整包测试覆盖配置、选择、Checker、日志、状态、更新及现有 Nix 集成；真实 PTY 检查正常退出、取消、渲染故障降级和终端恢复。
