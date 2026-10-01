{pin}: {
  release = pin.mk {
    checker = pin.checker.cmd "printf v1.2.3";
    sources = {
      repository = {
        fetcher = pin.fetcher.git {
          target = "https://example.com/demo.git";
          rev = version: "refs/tags/${version}";
        };
        patches = ["repository.patch"];
        postPatch = "echo repository";
        packages.api = pin.goModule {root = "cmd/api";};
      };
      archive = {
        fetcher = pin.fetcher.url {
          target = version: "https://example.com/demo-${version}.tar.gz";
        };
        packages.web = pin.npmPackage {root = "web";};
      };
    };
  };
}
