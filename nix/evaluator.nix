{
  pkgs,
  config,
  pins ? {},
  fake ? pkgs.lib.fakeHash,
}: let
  fetchers = import ./fetchers.nix {inherit pkgs fake;};
  builders = import ./builders.nix {inherit pkgs fake;};

  required = pinName: context: field: attrs:
    if builtins.hasAttr field attrs
    then builtins.getAttr field attrs
    else throw "nix-pins: pin '${pinName}' ${context} is missing field '${field}'";

  validateFields = pinName: context: allowed: attrs: let
    unknown = builtins.filter (name: !(builtins.elem name allowed)) (builtins.attrNames attrs);
  in
    if !builtins.isAttrs attrs
    then throw "nix-pins: pin '${pinName}' ${context} must be an attribute set"
    else if unknown == []
    then attrs
    else throw "nix-pins: pin '${pinName}' ${context} has unknown field '${builtins.head unknown}'";

  checker = {
    cmd = command: {
      _type = "cmd";
      args = {inherit command;};
    };
    github = args: {
      _type = "github";
      inherit args;
    };
    git = args: {
      _type = "git";
      inherit args;
    };
    crate = args: {
      _type = "crate";
      inherit args;
    };
    pypi = args: {
      _type = "pypi";
      inherit args;
    };
    npm = args: {
      _type = "npm";
      inherit args;
    };
    url = args: {
      _type = "url";
      inherit args;
    };
  };

  evaluateChecker = pinName: declaration: let
    kind = declaration._type or "unknown";
    rawArgs = declaration.args or {};
    compact = attrs:
      builtins.removeAttrs attrs (builtins.filter (name: attrs.${name} == null) (builtins.attrNames attrs));
  in
    if kind == "cmd"
    then let
      args = validateFields pinName "cmd Checker" ["command"] rawArgs;
    in {cmd = required pinName "cmd Checker" "command" args;}
    else if kind == "github"
    then let
      args = validateFields pinName "GitHub Checker" ["owner" "repo"] rawArgs;
      owner = required pinName "GitHub Checker" "owner" args;
      repo = required pinName "GitHub Checker" "repo" args;
    in {github = "${owner}/${repo}";}
    else if kind == "git"
    then let
      args = validateFields pinName "git Checker" ["url" "mode" "branch" "ref" "include" "exclude" "sort"] rawArgs;
      url = required pinName "git Checker" "url" args;
      mode = args.mode or "tag";
      branch = args.branch or null;
      ref = args.ref or null;
      include = args.include or null;
      exclude = args.exclude or null;
      sort = args.sort or "semver";
      value = compact {inherit url mode branch ref include exclude sort;};
    in
      if mode == "branch" && branch == null
      then throw "nix-pins: pin '${pinName}' git Checker mode 'branch' requires branch"
      else if mode == "ref" && ref == null
      then throw "nix-pins: pin '${pinName}' git Checker mode 'ref' requires ref"
      else if mode != "branch" && branch != null
      then throw "nix-pins: pin '${pinName}' git Checker branch is only valid in branch mode"
      else if mode != "ref" && ref != null
      then throw "nix-pins: pin '${pinName}' git Checker ref is only valid in ref mode"
      else {git = value;}
    else if kind == "crate"
    then let
      args = validateFields pinName "crate Checker" ["name"] rawArgs;
    in {crate = required pinName "crate Checker" "name" args;}
    else if kind == "pypi"
    then let
      args = validateFields pinName "PyPI Checker" ["name"] rawArgs;
    in {pypi = required pinName "PyPI Checker" "name" args;}
    else if kind == "npm"
    then let
      args = validateFields pinName "npm Checker" ["name" "distTag"] rawArgs;
    in {
      npm = {
        name = required pinName "npm Checker" "name" args;
        distTag = args.distTag or "latest";
      };
    }
    else if kind == "url"
    then let
      args = validateFields pinName "URL Checker" ["url" "regex"] rawArgs;
    in {
      url = {
        url = required pinName "URL Checker" "url" args;
        regex = required pinName "URL Checker" "regex" args;
      };
    }
    else throw "nix-pins: pin '${pinName}' uses unsupported Checker '${kind}'";

  mk = args: {
    _type = "pin";
    inherit args;
  };

  pin = {
    inherit checker mk;
    fetcher = fetchers.constructors;
    github = args: {
      _type = "github";
      inherit args;
    };
    inherit (builders) goModule npmPackage;
  };

  declarations = import config {inherit pin;};

  normalizePin = pinName: declaration:
    if declaration._type or null == "pin"
    then let
      args = validateFields pinName "pin.mk" ["checker" "fetcher" "packages"] declaration.args;
    in {
      checker = required pinName "pin.mk" "checker" args;
      fetcher = required pinName "pin.mk" "fetcher" args;
      packages = args.packages or {};
    }
    else if declaration._type or null == "github"
    then let
      args = validateFields pinName "pin.github" ["owner" "repo" "rev" "fetcherArgs" "packages"] declaration.args;
      checkerArgs =
        builtins.intersectAttrs {
          owner = null;
          repo = null;
        }
        args;
      fetcherArgs = builtins.removeAttrs args ["packages"];
    in {
      checker = checker.github checkerArgs;
      fetcher = fetchers.constructors.github fetcherArgs;
      packages = args.packages or {};
    }
    else throw "nix-pins: pin '${pinName}' uses unsupported fetcher '${declaration._type or "unknown"}'";

  evaluatePin = pinName: declaration: let
    normalized = normalizePin pinName declaration;
    check = evaluateChecker pinName normalized.checker;
    validationFetcher = (fetchers.evaluate pinName {version = "nix-pins-validation";} normalized.fetcher).fetcher;
    valid = builtins.deepSeq normalized.packages (builtins.deepSeq validationFetcher true);
    locked = pins.${pinName} or {};
    source = fetchers.evaluate pinName locked normalized.fetcher;
    packageNames = builtins.attrNames normalized.packages;
    baseHashName = packageName:
      builders.hashName pinName packageName normalized.packages.${packageName};
    hashName = packageName: let
      base = baseHashName packageName;
      sameType = builtins.filter (name: baseHashName name == base) packageNames;
    in
      if builtins.length sameType == 1
      then base
      else "${packageName}.${base}";
    packages =
      builtins.mapAttrs
      (packageName: builders.evaluate pinName packageName (hashName packageName) locked source.src)
      normalized.packages;
    derived =
      builtins.foldl'
      (result: packageName:
        result // builders.derived pinName packageName (hashName packageName) packages.${packageName})
      {}
      packageNames;
  in {
    check = assert valid; check;
    inherit (source) fetcher src;
    inherit derived packages;
  };
in
  builtins.mapAttrs evaluatePin declarations
