# 每项刻意违反一条声明约束，测试按名称选取；这些错误不是待修复的示例配置。
{pin}: {
  missing-owner = pin.github {
    repo = "demo";
    packages.default = pin.goModule {
      pname = "demo";
      root = ".";
    };
  };

  unsupported-fetcher = {
    _type = "gitlab";
    args.packages = {};
  };

  unsupported-builder = pin.github {
    target = "acme/demo";
    packages.default = {
      _type = "rustPackage";
      args.root = ".";
    };
  };

  reserved-fetcher-arg = pin.mk {
    checker = pin.checker.cmd "printf v1";
    fetcher = pin.fetcher.github {
      target = "acme/demo";
      fetcherArgs.owner = "override";
    };
  };

  non-string-url = pin.mk {
    checker = pin.checker.cmd "printf v1";
    fetcher = pin.fetcher.url {target = version: {inherit version;};};
  };

  invalid-github-target = pin.github {
    target = "acme/demo/extra";
  };

  conflicting-target = pin.mk {
    checker = pin.checker.cmd "printf v1";
    fetcher = pin.fetcher.git {
      target = "https://example.com/demo.git";
      url = "https://example.com/legacy.git";
    };
  };
}
