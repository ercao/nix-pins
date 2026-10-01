# 同一个 Pin 同时锁定 Go 与 npm 依赖。
{ pin }:
{
  cpa-manager-plus = pin.github {
    owner = "seakee";
    repo = "CPA-Manager-Plus";
    # 两个 Package 共用源码版本，按各自的依赖根目录分别计算派生哈希。
    packages = {
      manager-server = pin.goModule {
        pname = "cpa-manager-plus-manager-server";
        root = "apps/manager-server";
        preBuild = ''
          export GOPROXY=https://goproxy.cn,direct
        '';
      };

      web = pin.npmPackage {
        pname = "cpa-manager-plus-web";
        root = ".";
      };
    };
  };
}
