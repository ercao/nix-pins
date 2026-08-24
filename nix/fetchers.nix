{
  pkgs,
  fake,
}: let
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

  validateFetcherArgs = pinName: context: reserved: args: let
    fetcherArgs = args.fetcherArgs or {};
    reservedAttrs = builtins.listToAttrs (map (name: {
        inherit name;
        value = true;
      })
      reserved);
    conflicts = builtins.attrNames (builtins.intersectAttrs reservedAttrs fetcherArgs);
  in
    if !builtins.isAttrs fetcherArgs
    then throw "nix-pins: pin '${pinName}' ${context} fetcherArgs must be an attribute set"
    else if conflicts == []
    then true
    else throw "nix-pins: pin '${pinName}' ${context} fetcherArgs cannot override reserved field '${builtins.head conflicts}'";

  constructors = {
    github = args: {
      _type = "github";
      inherit args;
    };
    git = args: {
      _type = "git";
      inherit args;
    };
    huggingface = args: {
      _type = "huggingface";
      inherit args;
    };
    url = args: {
      _type = "url";
      inherit args;
    };
    zip = args: {
      _type = "zip";
      inherit args;
    };
  };

  validated = pinName: declaration: let
    kind = declaration._type or "unknown";
    rawArgs = declaration.args or {};
  in
    if kind == "github"
    then let
      args = validateFields pinName "GitHub Fetcher" ["target" "owner" "repo" "rev" "fetcherArgs"] rawArgs;
      target = targets.github pinName "GitHub Fetcher" args;
      inherit (target) owner repo;
      rev = args.rev or (version: version);
      validArgs = validateFetcherArgs pinName "GitHub Fetcher" ["owner" "repo" "rev" "hash"] args;
    in
      assert builtins.isFunction rev || throw "nix-pins: pin '${pinName}' GitHub Fetcher field 'rev' must be a function";
      assert validArgs; {inherit kind args owner repo rev;}
    else if kind == "git"
    then let
      args = validateFields pinName "git Fetcher" ["target" "url" "rev" "fetcherArgs"] rawArgs;
      url = targets.field pinName "git Fetcher" "url" args;
      rev = args.rev or (version: version);
      validArgs = validateFetcherArgs pinName "git Fetcher" ["url" "rev" "hash"] args;
    in
      assert builtins.isFunction rev || throw "nix-pins: pin '${pinName}' git Fetcher field 'rev' must be a function";
      assert validArgs; {inherit kind args url rev;}
    else if kind == "huggingface"
    then let
      args = validateFields pinName "Hugging Face Fetcher" ["target" "repoId" "rev" "fetcherArgs"] rawArgs;
      repoId = targets.field pinName "Hugging Face Fetcher" "repoId" args;
      rev = args.rev or (version: version);
      validArgs = validateFetcherArgs pinName "Hugging Face Fetcher" ["repoId" "rev" "tag" "hash"] args;
    in
      assert builtins.isString repoId || throw "nix-pins: pin '${pinName}' Hugging Face Fetcher field 'repoId' must be a string";
      assert builtins.isFunction rev || throw "nix-pins: pin '${pinName}' Hugging Face Fetcher field 'rev' must be a function";
      assert validArgs; {inherit kind args repoId rev;}
    else if kind == "url"
    then let
      args = validateFields pinName "URL Fetcher" ["target" "url" "fetcherArgs"] rawArgs;
      url = targets.field pinName "URL Fetcher" "url" args;
      validArgs = validateFetcherArgs pinName "URL Fetcher" ["url" "hash"] args;
    in
      assert builtins.isFunction url || throw "nix-pins: pin '${pinName}' URL Fetcher field 'url' must be a function";
      assert validArgs; {inherit kind args url;}
    else if kind == "zip"
    then let
      args = validateFields pinName "zip Fetcher" ["target" "url" "fetcherArgs"] rawArgs;
      url = targets.field pinName "zip Fetcher" "url" args;
      validArgs = validateFetcherArgs pinName "zip Fetcher" ["url" "hash"] args;
    in
      assert builtins.isFunction url || throw "nix-pins: pin '${pinName}' zip Fetcher field 'url' must be a function";
      assert validArgs; {inherit kind args url;}
    else throw "nix-pins: pin '${pinName}' uses unsupported Fetcher '${kind}'";

  mappedString = pinName: context: field: mapper: version: let
    value = mapper version;
  in
    if builtins.isString value
    then value
    else throw "nix-pins: pin '${pinName}' ${context} field '${field}' must map Version to a string";

  lockedVersion = pinName: locked:
    locked.version or (throw "nix-pins: pin '${pinName}' is missing a locked version");

  evaluate = pinName: locked: declaration: let
    config = validated pinName declaration;
    version = lockedVersion pinName locked;
    fetcherArgs = config.args.fetcherArgs or {};
  in
    if config.kind == "github"
    then let
      rev = mappedString pinName "GitHub Fetcher" "rev" config.rev version;
      fetcher = {
        github =
          fetcherArgs
          // {
            inherit (config) owner repo;
            inherit rev;
          };
      };
      src = builtins.deepSeq fetcher.github (pkgs.fetchFromGitHub (fetcher.github // {hash = locked.hash or fake;}));
    in {inherit fetcher src;}
    else if config.kind == "git"
    then let
      rev = mappedString pinName "git Fetcher" "rev" config.rev version;
      fetcher = {
        git =
          fetcherArgs
          // {
            inherit (config) url;
            inherit rev;
          };
      };
      src = builtins.deepSeq fetcher.git (pkgs.fetchgit (fetcher.git // {hash = locked.hash or fake;}));
    in {inherit fetcher src;}
    else if config.kind == "huggingface"
    then let
      rev = mappedString pinName "Hugging Face Fetcher" "rev" config.rev version;
      fetcher = {
        huggingface =
          {backend = "lfs";}
          // fetcherArgs
          // {
            inherit (config) repoId;
            inherit rev;
          };
      };
      src = builtins.deepSeq fetcher.huggingface (pkgs.fetchFromHuggingFace (fetcher.huggingface // {hash = locked.hash or fake;}));
    in {inherit fetcher src;}
    else if config.kind == "url"
    then let
      url = mappedString pinName "URL Fetcher" "url" config.url version;
      fetcher = {url = fetcherArgs // {inherit url;};};
      src = builtins.deepSeq fetcher.url (pkgs.fetchurl (fetcher.url // {hash = locked.hash or fake;}));
    in {inherit fetcher src;}
    else let
      url = mappedString pinName "zip Fetcher" "url" config.url version;
      fetcher = {zip = fetcherArgs // {inherit url;};};
      src = builtins.deepSeq fetcher.zip (pkgs.fetchzip (fetcher.zip // {hash = locked.hash or fake;}));
    in {inherit fetcher src;};
in {
  inherit constructors evaluate;
}
