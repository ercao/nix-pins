# TUI 状态与布局示例

在支持颜色、至少 60×12 的交互终端中运行：

```sh
cargo run --example tui
cargo run --example tui -- --list
cargo run --example tui -- --scene S04
```

默认 all 场景展示等待检查、等待下载、等待构建、检查、源码与派生求值、字节/对象进度、未知总量、补丁、复用、派生 Hash、成功、失败及 Pin/Source/Package 层级。数据固定，处理中图标动态刷新，不执行真实下载、求值、构建或 Pins File 保存。父级 nested 显示 v1 → v2，子任务共用该版本。

建议使用 160×34 查看总览；80×24 和 60×12 验证窄屏布局。j/k 滚动任务，[/] 滚动消息，q/Esc/Ctrl+C 退出示例并恢复终端，退出码为 0。生产 Update 的取消退出码为 130。

场景列表由 crates/cli/examples/tui 的 --list 输出提供。示例与渲染器测试共享 crates/cli/src/progress/scenes.rs，避免各自维护模拟状态。

## 保存最新截图

```sh
uv run --with pyte scripts/tui_examples_check.py
cargo build --bin nix-pins
uv run --with pyte scripts/tui_pipeline_check.py
```

- [全部状态与宽窄布局](../target/tui-validation/latest/examples/index.html)：验证全部可运行场景和 q/Esc/Ctrl+C。
- [独立 Pin 流水线](../target/tui-validation/latest/pipeline/index.html)：快 Pin 在慢 Source 下载时成功，释放后保存；取消恢复终端且不写文件。

两个脚本都支持 --output PATH；截图、文本帧和 JSON 检查结果统一保存在指定目录。默认 latest 目录本地保留并被 Git 忽略；没有生成报告时先运行对应命令。只保留当前实现的材料，临时浏览器回放页在脚本结束时自动删除。

截图来自真实 PTY 帧的 agent-browser 回放。状态示例使用固定数据，流水线使用受控 Nix 命令替身；这些结果不代表真实网络下载或 Nix 依赖构建通过。

## 测试闸门采集

需要检查测试驱动的终端关闭路径时运行：

```sh
uv run --with pyte scripts/tui_visual_check.py
```

它通过 ignored 的 capture_state 测试采集当前渲染器，检查结果保存在 target/tui-validation/latest/renderer。该入口与交互示例使用不同的结束协议，保留独立采集逻辑；文本读取和 HTML 回放复用现有函数。
