{ pkgs, fake }:
let
  required = pinName: packageName: field: attrs:
    if builtins.hasAttr field attrs then
      builtins.getAttr field attrs
    else
      throw "nix-pins: pin '${pinName}' package '${packageName}' is missing builder field '${field}'";

  lockedVersion = pinName: locked:
    locked.version or (throw "nix-pins: pin '${pinName}' is missing a locked version");
in
{
  goModule = args: {
    _type = "goModule";
    inherit args;
  };

  npmPackage = args: {
    _type = "npmPackage";
    inherit args;
  };

  hashName = pinName: packageName: declaration:
    if declaration._type or null == "goModule" then
      "vendorHash"
    else if declaration._type or null == "npmPackage" then
      "npmDepsHash"
    else
      throw "nix-pins: pin '${pinName}' package '${packageName}' uses unsupported builder '${declaration._type or "unknown"}'";

  evaluate = pinName: packageName: hashName: locked: src: declaration:
    let
      args = declaration.args or { };
      root = required pinName packageName "root" args;
      buildArgs = builtins.removeAttrs args [ "root" ];
      lockedDerived = locked.derived or { };
      hash = lockedDerived.${hashName} or fake;
      version = lockedVersion pinName locked;
    in
    if declaration._type or null == "goModule" then
      pkgs.buildGoModule (buildArgs // {
        inherit src version;
        modRoot = root;
        vendorHash = hash;
      })
    else if declaration._type or null == "npmPackage" then
      pkgs.buildNpmPackage (buildArgs // {
        inherit src version;
        npmRoot = root;
        npmDepsHash = hash;
      })
    else
      throw "nix-pins: pin '${pinName}' package '${packageName}' uses unsupported builder '${declaration._type or "unknown"}'";

  derived = pinName: packageName: hashName: package:
    if package ? goModules then
      { "${hashName}" = package.goModules.drvPath; }
    else if package ? npmDeps then
      { "${hashName}" = package.npmDeps.drvPath; }
    else
      throw "nix-pins: pin '${pinName}' package '${packageName}' did not expose a supported dependency FOD";
}
