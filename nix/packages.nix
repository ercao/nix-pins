{
  pkgs,
  config,
  pinsFile,
}: let
  data = builtins.fromJSON (builtins.readFile pinsFile);
  evaluated = import ./evaluator.nix {
    inherit pkgs config;
    pins = data.pins or (throw "nix-pins: pins file is missing 'pins'");
  };

  # 同类型 Intermediate FOD 唯一时提升到 Pin 顶层，便于 sources.pin.npmDeps 直接取用。
  withIntermediates = packages: let
    names = builtins.attrNames packages;
    go = builtins.filter (name: packages.${name} ? goModules) names;
    npm = builtins.filter (name: packages.${name} ? npmDeps) names;
  in
    packages
    // (
      if builtins.length go == 1
      then {goModules = packages.${builtins.head go}.goModules;}
      else {}
    )
    // (
      if builtins.length npm == 1
      then {npmDeps = packages.${builtins.head npm}.npmDeps;}
      else {}
    );
in
  if data.schemaVersion or null != 1
  then throw "nix-pins: unsupported pins file schema version"
  else builtins.mapAttrs (_: pin: withIntermediates pin.packages // {inherit (pin) src;}) evaluated
