{
  pkgs,
  fake,
}: let
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
    url = args: {
      _type = "url";
      inherit args;
    };
  };

  validated = pinName: declaration: let
    kind = declaration._type or "unknown";
    rawArgs = declaration.args or {};
  in
    if kind == "github"
    then let
      args = validateFields pinName "GitHub Fetcher" ["owner" "repo" "rev" "fetcherArgs"] rawArgs;
      owner = required pinName "GitHub Fetcher" "owner" args;
      repo = required pinName "GitHub Fetcher" "repo" args;
      rev = args.rev or (version: version);
      validArgs = validateFetcherArgs pinName "GitHub Fetcher" ["owner" "repo" "rev" "hash"] args;
    in
      assert builtins.isFunction rev || throw "nix-pins: pin '${pinName}' GitHub Fetcher field 'rev' must be a function";
      assert validArgs; {inherit kind args owner repo rev;}
    else if kind == "git"
    then let
      args = validateFields pinName "git Fetcher" ["url" "rev" "fetcherArgs"] rawArgs;
      url = required pinName "git Fetcher" "url" args;
      rev = args.rev or (version: version);
      validArgs = validateFetcherArgs pinName "git Fetcher" ["url" "rev" "hash"] args;
    in
      assert builtins.isFunction rev || throw "nix-pins: pin '${pinName}' git Fetcher field 'rev' must be a function";
      assert validArgs; {inherit kind args url rev;}
    else if kind == "url"
    then let
      args = validateFields pinName "URL Fetcher" ["url" "fetcherArgs"] rawArgs;
      url = required pinName "URL Fetcher" "url" args;
      validArgs = validateFetcherArgs pinName "URL Fetcher" ["url" "hash"] args;
    in
      assert builtins.isFunction url || throw "nix-pins: pin '${pinName}' URL Fetcher field 'url' must be a function";
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
    else let
      url = mappedString pinName "URL Fetcher" "url" config.url version;
      fetcher = {url = fetcherArgs // {inherit url;};};
      src = builtins.deepSeq fetcher.url (pkgs.fetchurl (fetcher.url // {hash = locked.hash or fake;}));
    in {inherit fetcher src;};
in {
  inherit constructors evaluate;
  validate = pinName: declaration: builtins.deepSeq (validated pinName declaration) true;
}
