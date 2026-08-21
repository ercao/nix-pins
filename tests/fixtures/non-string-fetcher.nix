{pin}: {
  demo = pin.mk {
    checker = pin.checker.cmd "printf should-not-run";
    fetcher = pin.fetcher.url {
      url = version: {inherit version;};
    };
  };
}
