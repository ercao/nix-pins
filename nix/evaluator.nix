{
  pkgs,
  config,
  pins ? {},
  fake ? pkgs.lib.fakeHash,
}: let
  fetchers = import ./fetchers.nix {inherit pkgs fake;};
  builders = import ./builders.nix {inherit pkgs fake;};
  pin = {
    inherit (fetchers) github;
    inherit (builders) goModule npmPackage;
  };
  declarations = import config {inherit pin;};

  evaluatePin = pinName: declaration: let
    locked = pins.${pinName} or {};
    source = fetchers.evaluate pinName locked declaration;
    packageNames = builtins.attrNames source.packages;
    baseHashName = packageName:
      builders.hashName pinName packageName source.packages.${packageName};
    hashName = packageName: let
      base = baseHashName packageName;
      sameType = builtins.filter (name: baseHashName name == base) packageNames;
    in
      if builtins.length sameType == 1
      then base
      else "${packageName}.${base}";
    packages =
      builtins.mapAttrs
      (packageName: builders.evaluate pinName packageName (hashName packageName) locked source.src)
      source.packages;
    derived =
      builtins.foldl'
      (result: packageName:
        result // builders.derived pinName packageName (hashName packageName) packages.${packageName})
      {}
      packageNames;
  in {
    inherit (source) check fetcher src;
    inherit derived packages;
  };
in
  builtins.mapAttrs evaluatePin declarations
