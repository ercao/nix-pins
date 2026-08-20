# npm 包必须支持透传 postPatch 以提供 lockfile

`buildNpmPackage` 的 `npmDeps` FOD 要求源码树中存在 `package-lock.json` 或 `npm-shrinkwrap.json`，否则构建在到达哈希比对之前即失败。实测 `svgo` 4.0.1 的 GitHub tarball 不含 lockfile：

```
svgo> ERROR: No lock file!
svgo> package-lock.json npm-shrinkwrap.json required
error: Cannot build '/nix/store/...-svgo-4.0.1-npm-deps.drv'. Reason: builder failed with exit code 1.
```

nixpkgs 对此的既定做法是在 `postPatch` 里 `cp` 一份单独 `fetchurl` 来的 lockfile（见 `pkgs/by-name/sl/sloc/package.nix`）。因此 npm Pin 的参数白名单必须包含 `postPatch`，且用户需要能在其中引用另一个被 fetch 的文件。

## Consequences

npm 支持无法做到「只给 owner/repo 就能算出 npmDepsHash」。缺少 lockfile 的仓库需要用户提供 postPatch，工具应在检测到该失败模式时给出指向此做法的明确提示，而不是只报告哈希解析失败。

这也是 ADR-0003 中「构建可能在到达哈希比对之前失败」这一解析器要求的真实实例。
