{pin}: {
  git-source = pin.mk {
    checker = pin.checker.cmd "printf v1.2.3";
    fetcher = pin.fetcher.git {
      url = "https://example.com/demo.git";
      rev = version: "refs/tags/${version}";
    };
  };

  url-source = pin.mk {
    checker = pin.checker.pypi {name = "demo";};
    fetcher = pin.fetcher.url {
      url = version: "https://example.com/demo-${version}.tar.gz";
    };
  };

  npm-source = pin.mk {
    checker = pin.checker.npm {
      name = "@scope/demo";
      distTag = "next";
    };
    fetcher = pin.fetcher.url {
      url = version: "https://registry.npmjs.org/@scope/demo/-/demo-${version}.tgz";
    };
  };

  git-checker-url-source = pin.mk {
    checker = pin.checker.git {
      url = "https://example.com/demo.git";
      mode = "branch";
      branch = "main";
    };
    fetcher = pin.fetcher.url {
      url = version: "https://example.com/demo-${version}.tar.gz";
    };
  };

  crate-git-source = pin.mk {
    checker = pin.checker.crate {name = "demo";};
    fetcher = pin.fetcher.git {
      url = "https://example.com/demo.git";
      rev = version: "refs/tags/${version}";
    };
  };
}
