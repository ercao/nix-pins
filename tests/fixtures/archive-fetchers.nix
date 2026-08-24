{pin}: {
  zip = pin.mk {
    checker = pin.checker.cmd "printf v1.2.3";
    fetcher = pin.fetcher.zip {
      target = version: "https://example.com/demo-${version}.tar.gz";
      fetcherArgs.stripRoot = false;
    };
  };

  huggingface = pin.mk {
    checker = pin.checker.cmd "printf 2.0.0";
    fetcher = pin.fetcher.huggingface {
      target = "acme/demo";
      rev = version: "refs/tags/${version}";
      fetcherArgs.repoType = "dataset";
    };
  };
}
