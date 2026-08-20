# 用真实上游 Checker 验证 macos-config 的 33 个 Pin；不写入 macos-config 工作树。
{ pkgs, fake, pins }:
let
  lib = pkgs.lib;
  configured = builtins.getEnv "MACOS_CONFIG_GENERATED_JSON";
  generated =
    if configured != "" then configured
    else "/Users/ercao/codes/macos-config/pkgs/_sources/generated.json";
  oldPins = builtins.fromJSON (builtins.readFile generated);
  gitHead = url: {
    cmd = ''
      attempts=0
      while [ "$attempts" -lt 3 ]; do
        output=$(git ls-remote ${lib.escapeShellArg url} HEAD) && [ -n "$output" ] && {
          printf '%s\n' "$output" | cut -f1
          exit 0
        }
        attempts=$((attempts + 1))
        sleep 1
      done
      exit 1
    '';
  };
  releaseRepositories = {
    "cli-proxy-api-management-center" = "router-for-me/Cli-Proxy-API-Management-Center";
    "cpa-usage-keeper" = "Willxup/cpa-usage-keeper";
    dws = "DingTalk-Real-AI/dingtalk-workspace-cli";
    mihomo = "MetaCubeX/mihomo";
  };
in
builtins.mapAttrs (
  name: old:
  let
    source = old.src;
    version = pins.${name}.version;
    releaseRepository = releaseRepositories.${name} or null;
  in
  {
    check =
      if source.type == "github" then {
        github = "${source.owner}/${source.repo}";
      } else if source.type == "git" then
        gitHead source.url
      else if releaseRepository != null then {
        github = releaseRepository;
      } else if name == "schemastore" then
        gitHead "https://github.com/SchemaStore/schemastore.git"
      else
        throw "nix-pins validation: no Checker for ${name}";

    src =
      if source.type == "github" then
        pkgs.fetchFromGitHub {
          inherit (source) owner repo;
          rev = version;
          hash = fake;
        }
      else if source.type == "git" then
        pkgs.fetchgit {
          inherit (source) url;
          rev = version;
          hash = fake;
        }
      else if name == "cli-proxy-api-management-center" then
        pkgs.fetchurl {
          url = "https://github.com/router-for-me/Cli-Proxy-API-Management-Center/releases/download/${version}/management.html";
          hash = fake;
        }
      else if name == "cpa-usage-keeper" then
        pkgs.fetchurl {
          url = "https://github.com/Willxup/cpa-usage-keeper/releases/download/${version}/cpa-usage-keeper_${version}_darwin_arm64.tar.gz";
          hash = fake;
        }
      else if name == "dws" then
        pkgs.fetchurl {
          url = "https://github.com/DingTalk-Real-AI/dingtalk-workspace-cli/releases/download/${version}/dws-darwin-arm64.tar.gz";
          hash = fake;
        }
      else if name == "mihomo" then
        pkgs.fetchurl {
          url = "https://github.com/MetaCubeX/mihomo/releases/download/${version}/mihomo-darwin-arm64-${version}.gz";
          hash = fake;
        }
      else if name == "schemastore" then
        pkgs.fetchurl {
          url = "https://raw.githubusercontent.com/SchemaStore/schemastore/${version}/src/api/json/catalog.json";
          hash = fake;
        }
      else
        throw "nix-pins validation: unsupported fetcher ${source.type} for ${name}";
  }
) oldPins
