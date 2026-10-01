# Pins File 保存 JSON，Pin 是原子更新边界

锁定结果只保存为 JSON Pins File，Nix Reader 将其转换为可用 Source 和 Package，避免维护第二份生成的 Nix 配置。schema v2 在 pins.<pin> 下保存一次 Version，在 sources.<source> 下保存带标签的 Fetcher、Hash、Derived Hash 与 Fingerprint；未使用的能力整个键缺失，键序稳定。Reader 不提升唯一 default Source 的属性，也不兼容 schema v1。

一个 Pin 拥有一个 Checker 和多个共享 Version 的 Source。任一 Source 或 Package 失败时保留整 Pin 的旧条目，首次失败不创建不完整 Pin；失败原因单独记录在 failures，其他成功 Pin 仍可更新，整轮以非零退出码报告部分失败。选择性 Update 保留范围外条目，完整 Update 清理配置中已不存在的条目。

更新事务在读取前取得目标身份对应的排他锁，持有至保存完成；内容写入同目录临时文件，同步后原子替换，字节不变时保留原文件和 mtime。取消不保存。这样既避免单个失败废弃昂贵的成功结果，也防止并发写者或中断产生半份 JSON。
