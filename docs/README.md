# nix-pins 文档网站

Docus 项目位于 `docs/`，采用官方 `pnpm create docus` 默认模板；公开页面来自 `content/`，站点检查脚本位于 `scripts/`。内部 ADR、规格、计划、原型与 TUI 验证说明位于仓库根目录的 `design/`，不参与网站构建。

在仓库根目录使用 Node.js 22.13.0 或更高版本与 pnpm 11.22.0。Nuxt Content 使用 Node 内置的 `node:sqlite` 驱动，不需要额外安装 `better-sqlite3`：

```sh
pnpm install --frozen-lockfile
pnpm docs:dev
```

`pnpm-workspace.yaml` 使用 `overrides` 取消 Docus 对 `better-sqlite3` 的必需 peer 要求。锁文件中仍可能保留 Content／db0 的可选 peer 引用；文档内容处理使用原生驱动。升级文档依赖时需重新核对原生驱动支持。

本地开发地址由 Nuxt 输出。生产构建与校验：

```sh
pnpm docs:build
pnpm docs:check
pnpm docs:examples
pnpm docs:examples --live
```

`docs:examples` 需要 Nix 和可解析的 `<nixpkgs>`。`--live` 还需要 Cargo、GitHub 网络访问和可写的 Nix store；它在临时目录运行 CLI，并实际构建 Reader 返回的锁定源码，不修改项目已有 Pins File。

静态产物位于 `docs/.output/public/`。生产站点地址为 `https://nix-pins.ercao.dev`，使用根路径 `/`。站点元数据与 `llms.txt` 共用该地址；可通过 `NUXT_SITE_URL` 覆盖。

Vercel 通过 Git 集成执行构建与链接检查，并发布生产与预览部署。根目录的 `vercel.json` 配置安装命令、静态构建与校验命令、产物目录和无 `.html` 后缀的页面路由。Nuxt 显式使用 `static` preset 固定产物目录，并关闭需要服务端的 AI 助手。

首次接入 Vercel：

1. 在 Vercel 导入 `ercao/nix-pins` 仓库，Production Branch 设为 `main`，Root Directory 使用仓库根目录 `.`，Framework Preset 选择 Other；构建设置由根目录的 `vercel.json` 提供。
2. Node.js Version 选择 `24.x`，环境变量 `ENABLE_EXPERIMENTAL_COREPACK` 设为 `1`，应用到 Production 与 Preview，让 Vercel 使用根目录 `packageManager` 指定的 pnpm 11.22.0。
3. 部署成功后，在项目 Settings → Domains 添加 `nix-pins.ercao.dev`。
4. 在 Cloudflare 的 `ercao.dev` → DNS → Records 中添加或更新 CNAME：Name 为 `nix-pins`，Target 使用 Vercel Domains 页面显示的值，Proxy status 为 DNS only，TTL 为 Auto。
5. 等待 Vercel 显示 Valid Configuration 并完成 HTTPS 证书签发后，访问 `https://nix-pins.ercao.dev`。

部署设置参见 [Vercel 配置文档](https://vercel.com/docs/project-configuration/vercel-json)、[Corepack 配置说明](https://vercel.com/kb/guide/how-do-i-use-the-latest-npm-version-for-my-vercel-deployment) 和 [自定义域名说明](https://vercel.com/docs/domains/working-with-domains/add-a-domain)。DNS 操作参见 [Cloudflare 文档](https://developers.cloudflare.com/dns/manage-dns-records/how-to/create-dns-records/)。

文档站接入 `@nuxtjs/i18n`，在 `nuxt.config.ts` 中配置语言。目前仅启用简体中文，路由代码与语言标识统一为 `zh-CN`；右上角使用 Lucide Languages 图标作为语言选择入口，菜单只有“简体中文”。中文内容位于 `content/zh-CN/`，页面使用 `/zh-CN/` 路径；不设置介绍首页；`/` 与 `/zh-CN` 均跳转到 `/zh-CN/getting-started/installation`，直接进入「安装」文档。四个目录入口（`getting-started`、`guides`、`reference`、`troubleshooting`）按导航顺序跳转到各自的第一篇文档，侧栏目录标题也可以点击；Vercel 额外将 `/getting-started` 转发到中文安装页。Content 的 `pathMeta.slugifyOptions.lower` 设为 `false`，保留路径中的语言代码大小写。

通过 Nuxt layer 的本地配置、页面与组件覆盖适配区域语言代码，不修改 Docus 依赖源码。`content.config.ts` 显式定义 `docs_zh_CN` 集合，`app/composables/useDocs.ts` 统一映射集合名与 Nuxt UI 的 `zh_cn` 语言包；目录与 URL 保留 `zh-CN`。语言入口使用 Docus 的 `AppHeaderCTA` 扩展点。搜索按当前语言查询集合并排除代码高亮的样式文本。新增语言时，需要同步添加内容目录、i18n 配置及对应集合；升级 Docus 时应验证这些本地覆盖。
