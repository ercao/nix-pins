# nix-pins docs

[Docus](https://docus.dev) documentation for nix-pins. English is the default language; Simplified Chinese is also available.

## Development

Requires Node.js 22.13.0+ and pnpm 11.22.0. Run from the repository root:

```sh
pnpm install --frozen-lockfile
pnpm docs:dev
```

Content lives in `content/en/` and `content/zh-CN/`, served under `/en/` and `/zh-CN/`. The root redirects to the English installation guide. Keep both languages in sync.

## Validation and deployment

```sh
pnpm docs:build
pnpm docs:check
pnpm docs:examples
```

Example checks require Nix and a resolvable `<nixpkgs>`. Add `--live` to `docs:examples` to run the quick start and build its locked source; this also requires Cargo, GitHub access, and a writable Nix store.

Static output is generated in `docs/.output/public/`. The root [`vercel.json`](../vercel.json) configures deployment. `NUXT_SITE_URL` overrides the default site URL, `https://nix-pins.ercao.dev`.

Nuxt Content uses Node's native SQLite driver. Keep `allowBuilds.better-sqlite3: false` in [`pnpm-workspace.yaml`](../pnpm-workspace.yaml) to skip the unused native build.
