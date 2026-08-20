# 用 macos-config 当前 nvfetcher 输出验证 33 个 Pin 的 Fetcher 与 src hash。
{ pkgs, fake, pins }:
let
  lib = pkgs.lib;
  configured = builtins.getEnv "MACOS_CONFIG_GENERATED_JSON";
  generated =
    if configured != ""
    then configured
    else "/Users/ercao/codes/macos-config/pkgs/_sources/generated.json";
  oldPins = builtins.fromJSON (builtins.readFile generated);
in
builtins.mapAttrs
  (name: old:
    let
      source = old.src;
      version = pins.${name}.version;
    in
    {
      # ticket 09 只验证迁移后的 Fetcher/hash；各内置 Checker 已由 tickets 06/08 覆盖。
      check.cmd = "printf %s " + lib.escapeShellArg old.version;
      src =
        if source.type == "github"
        then
          pkgs.fetchFromGitHub {
            inherit (source) owner repo;
            rev = version;
            hash = fake;
          }
        else if source.type == "git"
        then
          pkgs.fetchgit {
            inherit (source) url;
            rev = version;
            hash = fake;
          }
        else if source.type == "url"
        then
          pkgs.fetchurl {
            inherit (source) url;
            hash = fake;
          }
        else throw "nix-pins validation: unsupported fetcher ${source.type} for ${name}";
    })
  oldPins
