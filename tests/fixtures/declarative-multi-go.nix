{ pin }:
{
  demo = pin.github {
    owner = "acme";
    repo = "demo";
    packages = {
      api = pin.goModule {
        pname = "demo-api";
        root = "apps/api";
      };
      cli = pin.goModule {
        pname = "demo-cli";
        root = "apps/cli";
      };
    };
  };
}
