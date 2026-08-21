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
in
  if data.schemaVersion or null != 1
  then throw "nix-pins: unsupported pins file schema version"
  else builtins.mapAttrs (_: pin: pin.packages) evaluated
