# nix-pins reader —— 把 Pins File 转为可用的 src 与哈希属性（ADR-0002）。
{ pkgs, file ? ./pins.json }:
let
  data = builtins.fromJSON (builtins.readFile file);
  mkSrc = name: p:
    let f = p.fetcher; in
    if f ? github then pkgs.fetchFromGitHub (f.github // { inherit (p) hash; })
    else if f ? git then pkgs.fetchgit (f.git // { inherit (p) hash; })
    else if f ? url then pkgs.fetchurl (f.url // { inherit (p) hash; })
    else throw "nix-pins: unknown fetcher for ${name}";
in
builtins.mapAttrs
  (name: p: { inherit (p) version; src = mkSrc name p; } // (p.derived or {}))
  data.pins
