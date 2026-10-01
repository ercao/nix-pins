# 以 Pins File 中的真实版本和哈希重建配置，只向 Reader 暴露补丁源码与 Package。
{
  pkgs,
  config,
  pinsFile,
}: let
  data = builtins.fromJSON (builtins.readFile pinsFile);
  schemaVersion = data.schemaVersion or null;
  checked =
    if schemaVersion == 2
    then data
    else throw "nix-pins: unsupported Pins File schemaVersion ${toString schemaVersion}; expected 2";
  evaluated = import ./evaluator.nix {
    inherit pkgs config;
    pins = checked.pins or (throw "nix-pins: Pins File missing 'pins'");
  };
in
  builtins.mapAttrs
  (_: pin: {
    sources =
      builtins.mapAttrs
      (_: source: {
        inherit (source) src packages;
      })
      pin.sources;
  })
  evaluated
