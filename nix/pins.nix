# nix-pins Reader —— 将 schema v2 Pins File 转为严格的 Pin → Sources → Source 结构（ADR-0002、ADR-0021）。
# 传入 config 时，各 Source 额外暴露补丁后的 src、Package 与 Intermediate FOD。
{
  pkgs,
  file ? ../pins.json,
  config ? null,
}: let
  data = builtins.fromJSON (builtins.readFile file);
  schemaVersion = data.schemaVersion or null;
  checked =
    if schemaVersion == 2
    then data
    else throw "nix-pins: unsupported Pins File schemaVersion ${toString schemaVersion}; expected 2";
  mkSrc = pinName: sourceName: source: let
    fetcher = source.fetcher;
  in
    if fetcher ? github
    then pkgs.fetchFromGitHub (fetcher.github // {inherit (source) hash;})
    else if fetcher ? git
    then pkgs.fetchgit (fetcher.git // {inherit (source) hash;})
    else if fetcher ? huggingface
    then pkgs.fetchFromHuggingFace (fetcher.huggingface // {inherit (source) hash;})
    else if fetcher ? url
    then pkgs.fetchurl (fetcher.url // {inherit (source) hash;})
    else if fetcher ? zip
    then pkgs.fetchzip (fetcher.zip // {inherit (source) hash;})
    else throw "nix-pins: pin '${pinName}' source '${sourceName}' has unknown fetcher";
  locked =
    builtins.mapAttrs
    (pinName: pin: {
      inherit (pin) version;
      sources =
        builtins.mapAttrs
        (sourceName: source: {
          inherit (source) fetcher hash;
          derived = source.derived or {};
          src = mkSrc pinName sourceName source;
        })
        pin.sources;
    })
    checked.pins;
  evaluated =
    # 未传 config 时只还原锁定的原始源码；传入后才补充补丁源码和 Package。
    if config == null
    then {}
    else
      import ./packages.nix {
        inherit pkgs config;
        pinsFile = file;
      };
in
  builtins.mapAttrs
  (pinName: pin:
    pin
    // {
      sources =
        builtins.mapAttrs
        (sourceName: source:
          # 保留文件中的 Fetcher/哈希元数据，由配置求值结果覆盖 src 并追加 Package。
          source // (evaluated.${pinName}.sources.${sourceName} or {}))
        pin.sources;
    })
  locked
