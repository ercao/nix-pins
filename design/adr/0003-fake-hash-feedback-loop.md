# 真实 Nix 构建反馈求 Hash，drvPath 决定派生复用

Source Hash 与 Derived Hash 共用固定 SRI Fake Hash 的反馈循环，使用真实 Nix Fetcher 与 Builder 的输入语义，不在 Rust 中重写 prefetch 或语言生态的依赖抓取。只构建 Fetcher 输出或具名 Intermediate FOD：Go 的 goModules、npm 的 npmDeps、pnpm 的 pnpmDeps；不构建整个应用。

退出码不等价于取得 Hash。构建可能在 Hash 比对前失败，必须提取有效 Hash 或报告完整诊断，不能猜测结果；store 输出的保留取决于 Nix 和 GC，不作为持久缓存保证。

Source Hash 复用要求已解析的 Fetcher 参数一致；Derived Hash 复用比较带固定 Fake Hash 的 Intermediate FOD drvPath。该指纹由真实输入决定，捕获 Source、补丁、Builder 参数与 Nixpkgs 变化，避免手工参数白名单漏掉依赖输入。Probe 显式保留 Source、Package、Derived Hash 到 drvPath 的归属，不从扁平 Hash 键名反推 Package。

npm 需要 package-lock.json 或 npm-shrinkwrap.json；缺少时通过 Source 的 patches/postPatch 提供，依赖计算与下游使用同一份处理后的源码。pnpm 使用调用方的 pkgs.pnpm，允许显式覆盖；root 与正整数 fetcherVersion 必填，格式兼容性由调用方 Nixpkgs 决定，不读取全局 pnpm 或根据 package.json 自动选版本。工具不生成或修复锁文件，详见 [pnpm 接口](../specs/0008-pnpm-dependencies.md)。
