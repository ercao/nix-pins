# 应用自行绘制 Ratatui 任务视图，日志解析不决定业务结果

应用使用 Ratatui 与 Crossterm 绘制，Prodash 保留进度投影与最多 1024 条消息存储；不维护第三方 renderer 的源码补丁。Reporter 更新 Pin、Source、Package 状态，渲染线程在锁内复制快照后释放锁，业务执行不依赖终端刷新。

所有 nix build 使用 internal-json，nix/log.rs 负责解析与文本净化，提供文本、结构化 Counter、Activity 与 Message；执行层不依赖 progress。Pin Step 表示业务步骤，Nix 的下载、复制、构建、查询和 phase 仅补充详情；未知事件只能降低展示精度，不能改变 Hash 提取、错误判断或 Pins File。低频活动及诊断进入 Messages，高频计数不逐帧写入；URL 隐藏 userinfo 和 query，终端控制字符被净化。

左列显示动态图标、名称、版本和层级，按可见树内容宽度计算并为右列预留空间；滚动和动画不改变分栏位置。右列显示阶段与数值，只有已知正数总量才显示进度条；未知总量只保留累计值。等待灰色、处理中青色、成功绿色、失败红色加粗，动画只改变显示帧。

动态视图保留全部选中 Pin 并支持滚动，唯一 default Source/Package 可以折叠，成功 Package 折叠，失败分支保留。计算成功表示结果准备好，不表示已保存。Messages 和空间足够时的 Information 面板补充诊断与汇总；j/k 滚动任务，[/] 滚动消息。

process.rs 统一管理取消信号、子进程组与输出回收；q、Esc、Ctrl+C 单次取消整个 Update，停止工作并退出 130。正常退出、取消及绘制异常均恢复 alternate screen、光标和 raw mode；显示故障降级 plain，不改变业务结果。非 TTY、NO_COLOR、TERM=dumb 使用文本输出；离开动态界面后输出静态最终树与总结，失败或中断额外打印诊断，不整体复制 Messages 到 scrollback。

Reporter/TestBackend 验证状态及布局，真实 PTY 验证终端恢复、滚动、缩放和取消。最新截图位于 target/tui-validation/latest，是受控数据的 PTY 帧回放；使用入口见 [TUI 示例](../tui-examples.md)。
