# nix-pins 声明式配置示例。
{ pin }:
let
  # npmDeps 要求源码树含 lockfile；sloc 的 tarball 不含，需单独取（ADR-0012）。
  slocLock = builtins.fetchurl {
    url = "https://raw.githubusercontent.com/flosse/sloc/e26044011821c4e170859362f2de657d64118711/package-lock.json";
    sha256 = "sha256-yWVErql5SWOSbbw2DZUlXBJp7zqZngkc8uAC5ZfnjX0=";
  };
in
{
  curlie = pin.github {
    owner = "rs";
    repo = "curlie";
    packages.default = pin.goModule {
      pname = "curlie";
      root = ".";
      ldflags = [ "-s" "-w" ];
    };
  };

  sloc = pin.github {
    owner = "flosse";
    repo = "sloc";
    packages.default = pin.npmPackage {
      pname = "sloc";
      root = ".";
      postPatch = "cp ${slocLock} ./package-lock.json";
    };
  };
}
