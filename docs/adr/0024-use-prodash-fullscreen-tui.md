---
status: accepted
---

# TTY 使用 Prodash 全屏 TUI

交互式 Update 在 TTY 下使用 Prodash 官方 `render-tui`，运行期间进入 alternate screen 并接管键盘；命令结束后恢复 shell，并从进度状态输出静态最终摘要。非 TTY 与 CI 继续输出 plain 文本。选择全屏 TUI 是为了获得统一的视觉层级、更高的信息密度和官方支持的任务、消息及滚动布局；本决策取代 ADR-0019 与 ADR-0023 中仅针对 TTY renderer、alternate screen 和键盘接管的限制，其余进度领域模型与最终输出语义继续有效。

TUI 使用官方上下分区：上方 Activity Feed 展示 Nix 活动信息，下方展示 Pin、Source 与 Package 进度树。Activity Feed 完整显示 Store Path、drvPath、构建阶段与普通 Nix 消息；URL 保留 host/path，但隐藏 userinfo 与 query 参数。

Activity Feed 负责 Build、Copy、Query 等活动历史；下方进度树只在最深可见的 Pin、Source 或 Package 节点显示当前状态或结构化计数器，不创建重复的活动子行。

Activity Feed 只记录开始构建、stdenv phase、复制、查询及 warning/error 等低频状态变化和诊断。下载字节、Git/LFS 对象百分比只更新进度树中的结构化计数器，不逐帧写入 Feed。

Ctrl+C、`q` 与 Esc 均单次取消整个 Update：停止正在运行的工作、恢复终端，并以退出码 130 结束。不采用二次确认或双阶段强制退出。

退出 alternate screen 后，成功命令只打印静态最终树与总结；失败或中断额外打印 warning/error 消息。完整 Activity Feed 仅在 TUI 运行期间展示，不复制到 scrollback。

进度树展示全部选中的 Pin，并使用 Prodash 官方 `j`/`k` 滚动。成功 Pin 保留，成功 Package 折叠，活动与失败分支展开；移除 line renderer 为终端高度设置的 Pin 可见性软预算。

TUI 是 best-effort 展示。初始化失败、终端能力不足或渲染异常时，自动降级为与非 TTY 相同的 plain 文本输出并继续 Update；不保留旧 line renderer 作为备用动态后端。

启用 Prodash 官方 Information 面板，显示当前全局阶段、Pin 完成/总数与失败数、活跃 Build/Download/Copy/Query 数量，以及 `j`/`k` 和 `q`/Esc/Ctrl+C 快捷键。详细事件只出现在 Activity Feed。

终端空间不足时优先保留进度树，依次隐藏 Information 面板、压缩 Activity Feed；仍无法可靠展示时降级 plain，避免输出严重截断的 TUI。
