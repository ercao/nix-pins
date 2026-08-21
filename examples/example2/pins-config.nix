# 同一个 Pin 同时锁定 Go 与 npm 依赖。
{ pin }:
{
  cpa-manager-plus = pin.github {
    owner = "seakee";
    repo = "CPA-Manager-Plus";
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
