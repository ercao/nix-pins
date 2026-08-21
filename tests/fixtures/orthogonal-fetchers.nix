{pin}: {
  git-default = pin.mk {
    checker = pin.checker.cmd "printf v1.2.3";
    fetcher = pin.fetcher.git {
      url = "https://example.com/default.git";
    };
  };

  git-mapped = pin.mk {
    checker = pin.checker.crate {name = "demo";};
    fetcher = pin.fetcher.git {
      url = "https://example.com/mapped.git";
      rev = version: "refs/tags/${version}";
    };
  };

  url = pin.mk {
    checker = pin.checker.pypi {name = "demo";};
    fetcher = pin.fetcher.url {
      url = version: "https://example.com/demo-${version}.tar.gz";
    };
  };
}
