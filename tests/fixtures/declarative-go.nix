{ pin }:
{
  demo = pin.github {
    owner = "acme";
    repo = "demo";
    packages.default = pin.goModule {
      pname = "demo";
      root = "cmd/demo";
      ldflags = [ "-s" ];
    };
  };
}
