# nix-pins reader —— 把 Pins File 转为可用的 src 与哈希属性（ADR-0002）。
# 传入 config 时，额外合并处理后的 src、包与 Intermediate FOD（如 npmDeps）。
{
  pkgs,
  file ? ./pins.json,
  config ? null,
}: let
  data = builtins.fromJSON (builtins.readFile file);
  mkSrc = name: p: let
    f = p.fetcher;
  in
    if f ? github
    then pkgs.fetchFromGitHub (f.github // {inherit (p) hash;})
    else if f ? git
    then pkgs.fetchgit (f.git // {inherit (p) hash;})
    else if f ? url
    then pkgs.fetchurl (f.url // {inherit (p) hash;})
    else throw "nix-pins: unknown fetcher for ${name}";

  pins =
    builtins.mapAttrs (
      name: p: {pname = name; inherit (p) version; src = mkSrc name p;} // (p.derived or {})
    )
    data.pins;

  packages =
    if config == null
    then {}
    else
      import ./nix/packages.nix {
        inherit pkgs config;
        pinsFile = file;
      };
in
  builtins.mapAttrs (name: pin: pin // (packages.${name} or {})) pins
