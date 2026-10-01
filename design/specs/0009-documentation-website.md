# nix-pins 文档网站

状态：网站已实现，本地验收通过；Vercel 部署与域名绑定尚未执行。

## 目标读者

已经会使用 Nix、第一次使用 nix-pins 的开发者。入门内容不承担 Nix 基础教学。

## 首版范围

- 入门教程：从安装到完成一次实际使用。
- 使用指南：更新、选择性更新和常见使用场景。
- 参考文档：CLI、配置和公开 Nix API。
- 排错指南：常见错误与处理方法。

内部架构和贡献指南留待后续补充。

## 内容组织

- 首页：项目用途与快速开始入口。
- 快速开始：安装、声明 Pin、Update、生成 Pins File、通过 Reader 在 Nix 中使用结果。
- 使用指南：常见更新场景、Checker 与 Fetcher 组合、依赖哈希、多 Source、多 Package。
- 参考文档：CLI、运行配置、Pin 声明、Checker、Fetcher、Package Builder、Reader。
- 排错指南：配置错误、版本检查错误、获取来源与依赖哈希错误，以及定位方法。

Nix API 参考按 Checker、Fetcher、Package Builder 的职责分层；Pin 声明和 Reader 分别说明如何组合声明和消费结果。

## 能力覆盖与教程示例

文档覆盖全部公开能力。GitHub 源码 Pin 仅作为快速开始的贯穿示例，不限定网站的内容范围。

按当前公开构造器核对，参考文档需要覆盖：

| 能力 | 公开入口 |
| --- | --- |
| Checker：版本检查 | `cmd`、`github`、`git`、`crate`、`pypi`、`npm`、`url` |
| Fetcher：获取来源 | `github`、`git`、`huggingface`、`url`、`zip` |
| Package Builder：依赖产物与派生哈希 | `goModule`、`npmPackage`、`pnpmPackage` |
| Pin 声明 | `pin.mk`、`pin.github`、`pin.git` |

Checker 与 Fetcher 分别负责版本检查和获取来源，文档按其职责说明可组合的用法。依赖哈希、多 Source 和多 Package 放入进阶指南；各类公开入口均提供对应说明与示例。

示例以当前 CLI、公开 Nix API 和 Reader 为准，在实施阶段运行校验。

## 文档语言

首版使用中文，英文版本后续补充。先完善中文内容，避免在内容尚未稳定时同步维护两套文本。

## 网站框架

使用 Docus，满足 Vue 生态的选型约束，并采用其 Nuxt 文档网站方案。

## 发布方案

按静态站点构建，发布到 Vercel，绑定由 Cloudflare 管理的 `nix-pins.ercao.dev`，使用根路径 `/`。默认站点地址为 `https://nix-pins.ercao.dev`，可通过 `NUXT_SITE_URL` 覆盖。网站源码随项目仓库维护。

## 实施安排

Docus 项目根目录为 `docs/`，面向使用者的页面放入 `docs/content/`，使用默认文档布局。仓库根级 pnpm workspace 只纳入 `docs`，根目录统一管理 pnpm 版本、锁文件和文档命令，Docus 配置与依赖声明位于 `docs/`。内部设计文档统一放入 `design/`，包括 `design/adr/`、`design/specs/` 与 `design/plans/`；网站内容仅从 `docs/content/` 组织。

## 验收标准

- 静态构建成功，并检查生成页面的内部链接。
- 快速开始示例实际执行，核对 CLI、Pins File 与 Reader 输出；参考示例检查声明是否符合当前公开 API。
- 用浏览器检查导航、搜索、代码块及移动端布局。
- 配置 Vercel 发布流程，并检查独立域名根路径下的页面和资源链接。

已完成本地验证：

- 15 个公开 Markdown 页面完成静态构建；17 个 HTML（含 200/404）与 2065 个站内链接／资源检查通过。
- 27 个完整 Pin 声明通过当前 Nix API 求值；真实快速开始锁定 patchelf 0.19.1，生成 schema v2 Pins File，并成功构建 Reader 返回的源码。
- 桌面与 390×844 移动端检查导航、搜索结果跳转和代码复制；移动端无页面横向溢出，浏览器无控制台错误。
- 根级 pnpm workspace 的冻结锁文件安装通过，已配置 Vercel 静态部署；构建与链接检查统一由 Vercel 执行。
- 模拟 Vercel 环境的静态构建通过，产物固定为 `docs/.output/public`，AI 助手保持关闭；`vercel.json` 使用的字段按官方 schema 定义校验通过。实际云端部署尚未执行。

Vue 类型检查未在项目源码中发现错误，但完整检查仍有 Docus 依赖源码的类型错误；Serena 当前 TypeScript 服务不能正确解析 Vue SFC，不能将其诊断当作 Vue 语义验证。网站尚未发布，Vercel 项目接入、自定义域名与 Cloudflare DNS 尚未设置。

## 领域术语

沿用根目录 [CONTEXT.md](../../CONTEXT.md) 中的领域术语。讨论中确定新的领域概念时，在该文件中维护定义。
