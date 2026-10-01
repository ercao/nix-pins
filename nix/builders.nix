# 将 Package 声明接到 nixpkgs builder，暴露依赖 FOD 而不构建最终应用。
{
  pkgs,
  fake,
}: let
  required = pinName: packageName: field: attrs:
    if builtins.hasAttr field attrs
    then builtins.getAttr field attrs
    else throw "nix-pins: pin '${pinName}' package '${packageName}' is missing builder field '${field}'";

  lockedVersion = pinName: locked:
    locked.version or (throw "nix-pins: pin '${pinName}' is missing a locked version");

  hashName = pinName: packageName: declaration:
    if declaration._type or null == "goModule"
    then "vendorHash"
    else if declaration._type or null == "npmPackage"
    then "npmDepsHash"
    else if declaration._type or null == "pnpmPackage"
    then "pnpmDepsHash"
    else throw "nix-pins: pin '${pinName}' package '${packageName}' uses unsupported builder '${declaration._type or "unknown"}'";

  # 依赖哈希来自 builder 暴露的中间 FOD，不从最终应用的构建日志猜测。
  intermediateName = pinName: packageName: declaration:
    if declaration._type or null == "goModule"
    then "goModules"
    else if declaration._type or null == "npmPackage"
    then "npmDeps"
    else if declaration._type or null == "pnpmPackage"
    then "pnpmDeps"
    else throw "nix-pins: pin '${pinName}' package '${packageName}' uses unsupported builder '${declaration._type or "unknown"}'";
in {
  goModule = args: {
    _type = "goModule";
    inherit args;
  };

  npmPackage = args: {
    _type = "npmPackage";
    inherit args;
  };

  pnpmPackage = args: {
    _type = "pnpmPackage";
    inherit args;
  };

  inherit hashName intermediateName;

  # pnpm 当前锁定整个声明的 workspace，提前拒绝会改变依赖集合的过滤参数。
  validate = pinName: packageName: declaration:
    if declaration._type or null == "pnpmPackage"
    then let
      root = required pinName packageName "root" declaration.args;
      fetcherVersion = required pinName packageName "fetcherVersion" declaration.args;
      filtersWorkspace = flag:
        builtins.elem flag ["--filter" "--filter-prod"]
        || pkgs.lib.hasPrefix "--filter=" flag
        || pkgs.lib.hasPrefix "--filter-prod=" flag
        || pkgs.lib.hasPrefix "-F" flag;
    in
      assert builtins.isString root && root != ""
        || throw "nix-pins: pin '${pinName}' package '${packageName}' requires 'root' to be a nonempty string";
      assert builtins.isInt fetcherVersion && fetcherVersion > 0
        || throw "nix-pins: pin '${pinName}' package '${packageName}' requires 'fetcherVersion' to be a positive integer";
      assert (declaration.args.pnpmWorkspaces or []) == []
        || throw "nix-pins: pin '${pinName}' package '${packageName}' does not support filtering packages with 'pnpmWorkspaces'";
      assert !(builtins.any filtersWorkspace (declaration.args.pnpmInstallFlags or []))
        || throw "nix-pins: pin '${pinName}' package '${packageName}' does not support workspace filtering in 'pnpmInstallFlags'";
      true
    else true;

  evaluate = pinName: packageName: hashName: locked: src: declaration: let
    args = declaration.args or {};
    root = required pinName packageName "root" args;
    buildArgs =
      {
        pname =
          if packageName == "default"
          then pinName
          else "${pinName}-${packageName}";
      }
      // removeAttrs args ["root"];
    lockedDerived = locked.derived or {};
    # 首轮 Probe 使用固定占位哈希；Reader 则注入已经解析出的真实哈希。
    hash = lockedDerived.${hashName} or fake;
    version = lockedVersion pinName locked;
  in
    if declaration._type or null == "goModule"
    then
      pkgs.buildGoModule (buildArgs
        // {
          inherit src version;
          modRoot = root;
          vendorHash = hash;
        })
    else if declaration._type or null == "npmPackage"
    then
      pkgs.buildNpmPackage (buildArgs
        // {
          inherit src version;
          npmRoot = root;
          npmDepsHash = hash;
        })
    else if declaration._type or null == "pnpmPackage"
    then let
      pnpmDeps = pkgs.fetchPnpmDeps (buildArgs
        // {
          inherit src version hash;
          pnpm = args.pnpm or pkgs.pnpm;
          fetcherVersion = required pinName packageName "fetcherVersion" args;
          postUnpack = (args.postUnpack or "") + ''

            sourceRoot="$sourceRoot"/${pkgs.lib.escapeShellArg root}
          '';
        });
    in
      # fetchPnpmDeps 自身就是依赖 FOD；补齐统一的 pnpmDeps 访问入口。
      pnpmDeps // {inherit pnpmDeps;}
    else throw "nix-pins: pin '${pinName}' package '${packageName}' uses unsupported builder '${declaration._type or "unknown"}'";

  derived = pinName: packageName: hashName: declaration: package:
    let
      intermediate = intermediateName pinName packageName declaration;
    in {"${hashName}" = package.${intermediate}.drvPath;};
}
