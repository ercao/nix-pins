# mapper 刻意返回属性集，验证 Fetcher 类型错误会先于 should-not-run Checker 被报告。
{pin}: {
  demo = pin.mk {
    checker = pin.checker.cmd "printf should-not-run";
    fetcher = pin.fetcher.url {
      target = version: {inherit version;};
    };
  };
}
