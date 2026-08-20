# nix-pins 配置示例。工具注入 pkgs、fake、pins 三个参数（ADR-0013、ADR-0015）。
#
# 约束：check 不得依赖 pins —— 阶段一以空 pins 求值它（ADR-0015）。
{ pkgs, fake, pins }:
let
  # npmDeps 要求源码树含 lockfile；sloc 的 tarball 不含，需单独取（ADR-0012）。
  slocLock = pkgs.fetchurl {
    url = "https://raw.githubusercontent.com/flosse/sloc/e26044011821c4e170859362f2de657d64118711/package-lock.json";
    hash = "sha256-yWVErql5SWOSbbw2DZUlXBJp7zqZngkc8uAC5ZfnjX0=";
  };
in
{
  curlie = {
    check.github = "rs/curlie";
    src = pkgs.fetchFromGitHub {
      owner = "rs";
      repo = "curlie";
      rev = pins.curlie.version;
      hash = fake;
    };
    derive = src: pkgs.buildGoModule {
      pname = "curlie";
      version = pins.curlie.version;
      inherit src;
      vendorHash = fake;
      ldflags = [ "-s" "-w" ];
    };
  };

  sloc = {
    check.github = "flosse/sloc";
    src = pkgs.fetchFromGitHub {
      owner = "flosse";
      repo = "sloc";
      rev = pins.sloc.version;
      hash = fake;
    };
    derive = src: pkgs.buildNpmPackage {
      pname = "sloc";
      version = pins.sloc.version;
      inherit src;
      npmDepsHash = fake;
      postPatch = "cp " + slocLock + " ./package-lock.json";
    };
  };
}
