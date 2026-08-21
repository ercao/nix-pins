{ pin }:
{
  missing-owner = pin.github {
    repo = "demo";
    packages.default = pin.goModule {
      pname = "demo";
      root = ".";
    };
  };

  unsupported-fetcher = {
    _type = "gitlab";
    args.packages = { };
  };

  unsupported-builder = pin.github {
    owner = "acme";
    repo = "demo";
    packages.default = {
      _type = "rustPackage";
      args.root = ".";
    };
  };
}
