{pin}: {
  git-default = pin.mk {
    checker = pin.checker.cmd "printf v1.2.3";
    fetcher = pin.fetcher.git {
      target = "https://example.com/default.git";
    };
  };

  git-mapped = pin.mk {
    checker = pin.checker.crate {target = "demo";};
    fetcher = pin.fetcher.git {
      target = "https://example.com/mapped.git";
      rev = version: "refs/tags/${version}";
    };
  };

  url = pin.mk {
    checker = pin.checker.pypi {target = "demo";};
    fetcher = pin.fetcher.url {
      target = version: "https://example.com/demo-${version}.tar.gz";
    };
  };
}
