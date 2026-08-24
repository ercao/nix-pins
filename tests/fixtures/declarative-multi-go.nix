{pin}: {
  demo = pin.github {
    target = "acme/demo";
    packages = {
      api = pin.goModule {
        root = "apps/api";
      };
      cli = pin.goModule {
        root = "apps/cli";
      };
    };
  };
}
