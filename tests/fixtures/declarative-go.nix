{
  pin,
  pkgs,
  lib,
}:
assert lib.fakeHash == pkgs.lib.fakeHash; {
  demo = pin.github {
    target = "acme/demo";
    patches = ["demo.patch"];
    postPatch = "echo patched";
    packages.default = pin.goModule {
      root = "cmd/demo";
      ldflags = ["-s"];
    };
  };
}
