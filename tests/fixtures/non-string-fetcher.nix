{pin}: {
  demo = pin.mk {
    checker = pin.checker.cmd "printf should-not-run";
    fetcher = pin.fetcher.url {
      target = version: {inherit version;};
    };
  };
}
