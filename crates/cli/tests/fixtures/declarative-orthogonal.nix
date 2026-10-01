{pin}: {
  git-source = pin.mk {
    checker = pin.checker.cmd "printf v1.2.3";
    fetcher = pin.fetcher.git {
      target = "https://example.com/demo.git";
      rev = version: "refs/tags/${version}";
    };
  };

  url-source = pin.mk {
    checker = pin.checker.pypi {target = "demo";};
    fetcher = pin.fetcher.url {
      target = version: "https://example.com/demo-${version}.tar.gz";
    };
  };

  npm-source = pin.mk {
    checker = pin.checker.npm {
      target = "@scope/demo";
      distTag = "next";
    };
    fetcher = pin.fetcher.url {
      target = version: "https://registry.npmjs.org/@scope/demo/-/demo-${version}.tgz";
    };
  };

  git-checker-url-source = pin.mk {
    checker = pin.checker.git {
      target = "https://example.com/demo.git";
      mode = "branch";
      branch = "main";
    };
    fetcher = pin.fetcher.url {
      target = version: "https://example.com/demo-${version}.tar.gz";
    };
  };

  crate-git-source = pin.mk {
    checker = pin.checker.crate {target = "demo";};
    fetcher = pin.fetcher.git {
      target = "https://example.com/demo.git";
      rev = version: "refs/tags/${version}";
    };
  };
}
