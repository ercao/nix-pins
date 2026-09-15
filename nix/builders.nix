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
        || throw "nix-pins: pin '${pinName}' package '${packageName}' 的 'root' 必须为非空字符串";
      assert builtins.isInt fetcherVersion && fetcherVersion > 0
        || throw "nix-pins: pin '${pinName}' package '${packageName}' 的 'fetcherVersion' 必须为正整数";
      assert (declaration.args.pnpmWorkspaces or []) == []
        || throw "nix-pins: pin '${pinName}' package '${packageName}' 暂不支持通过 'pnpmWorkspaces' 过滤子包";
      assert !(builtins.any filtersWorkspace (declaration.args.pnpmInstallFlags or []))
        || throw "nix-pins: pin '${pinName}' package '${packageName}' 暂不支持在 'pnpmInstallFlags' 中过滤 workspace 子包";
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
      pnpmDeps // {inherit pnpmDeps;}
    else throw "nix-pins: pin '${pinName}' package '${packageName}' uses unsupported builder '${declaration._type or "unknown"}'";

  derived = pinName: packageName: hashName: declaration: package:
    let
      intermediate = intermediateName pinName packageName declaration;
    in {"${hashName}" = package.${intermediate}.drvPath;};
}
