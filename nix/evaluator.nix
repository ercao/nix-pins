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

  targets = import ./targets.nix {inherit required;};

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
      args = validateFields pinName "GitHub Checker" ["target" "owner" "repo"] rawArgs;
      target = targets.github pinName "GitHub Checker" args;
      inherit (target) owner repo;
    in {github = "${owner}/${repo}";}
    else if kind == "git"
    then let
      args = validateFields pinName "git Checker" ["target" "url" "mode" "branch" "ref" "include" "exclude" "sort"] rawArgs;
      url = targets.field pinName "git Checker" "url" args;
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
      args = validateFields pinName "crate Checker" ["target" "name"] rawArgs;
    in {crate = targets.field pinName "crate Checker" "name" args;}
    else if kind == "pypi"
    then let
      args = validateFields pinName "PyPI Checker" ["target" "name"] rawArgs;
    in {pypi = targets.field pinName "PyPI Checker" "name" args;}
    else if kind == "npm"
    then let
      args = validateFields pinName "npm Checker" ["target" "name" "distTag"] rawArgs;
    in {
      npm = {
        name = targets.field pinName "npm Checker" "name" args;
        distTag = args.distTag or "latest";
      };
    }
    else if kind == "url"
    then let
      args = validateFields pinName "URL Checker" ["target" "url" "regex"] rawArgs;
    in {
      url = {
        url = targets.field pinName "URL Checker" "url" args;
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

  configFunction = import config;
  configArguments = {
    inherit pin pkgs;
    lib = pkgs.lib;
  };
  declarations = configFunction (builtins.intersectAttrs (builtins.functionArgs configFunction) configArguments);

  normalizePin = pinName: declaration:
    if declaration._type or null == "pin"
    then let
      args = validateFields pinName "pin.mk" ["checker" "fetcher" "patches" "postPatch" "packages"] declaration.args;
    in {
      checker = required pinName "pin.mk" "checker" args;
      fetcher = required pinName "pin.mk" "fetcher" args;
      patches = args.patches or [];
      postPatch = args.postPatch or "";
      packages = args.packages or {};
    }
    else if declaration._type or null == "github"
    then let
      args = validateFields pinName "pin.github" ["target" "owner" "repo" "rev" "fetcherArgs" "patches" "postPatch" "packages"] declaration.args;
      target = targets.github pinName "pin.github" args;
      checkerArgs = target;
      fetcherArgs = builtins.removeAttrs args ["target" "owner" "repo" "patches" "postPatch" "packages"] // target;
    in {
      checker = checker.github checkerArgs;
      fetcher = fetchers.constructors.github fetcherArgs;
      patches = args.patches or [];
      postPatch = args.postPatch or "";
      packages = args.packages or {};
    }
    else throw "nix-pins: pin '${pinName}' uses unsupported fetcher '${declaration._type or "unknown"}'";

  evaluatePin = pinName: declaration: let
    normalized = normalizePin pinName declaration;
    check = evaluateChecker pinName normalized.checker;
    validationFetcher = (fetchers.evaluate pinName {version = "nix-pins-validation";} normalized.fetcher).fetcher;
    valid = builtins.deepSeq normalized.packages (builtins.deepSeq validationFetcher true);
    locked = pins.${pinName} or {};
    fetched = fetchers.evaluate pinName locked normalized.fetcher;
    src =
      if normalized.patches == [] && normalized.postPatch == ""
      then fetched.src
      else
        pkgs.applyPatches {
          name = "${pinName}-patched";
          src = fetched.src;
          inherit (normalized) patches postPatch;
        };
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
        (packageName: builders.evaluate pinName packageName (hashName packageName) locked src)
        normalized.packages;
      packageDerived =
        builtins.mapAttrs
        (packageName: _: builders.derived pinName packageName (hashName packageName) normalized.packages.${packageName} packages.${packageName})
        normalized.packages;
      derived =
        builtins.foldl'
        (result: packageName: result // packageDerived.${packageName})
        {}
        packageNames;
  in {
    check = assert valid; check;
    inherit (fetched) fetcher;
    fetchSrc = fetched.src;
    inherit src;
      inherit derived packageDerived packages;
  };
in
  builtins.mapAttrs evaluatePin declarations
