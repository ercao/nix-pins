{pin}: {
  missing-owner = pin.github {
    repo = "demo";
    packages.default = pin.goModule {
      pname = "demo";
      root = ".";
    };
  };

  unsupported-fetcher = {
    _type = "gitlab";
    args.packages = {};
  };

  unsupported-builder = pin.github {
    owner = "acme";
    repo = "demo";
    packages.default = {
      _type = "rustPackage";
      args.root = ".";
    };
  };

  reserved-fetcher-arg = pin.mk {
    checker = pin.checker.cmd "printf v1";
    fetcher = pin.fetcher.github {
      owner = "acme";
      repo = "demo";
      fetcherArgs.owner = "override";
    };
  };

  non-string-url = pin.mk {
    checker = pin.checker.cmd "printf v1";
    fetcher = pin.fetcher.url {url = version: {inherit version;};};
  };
}
