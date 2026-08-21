{
  pkgs,
  fake,
}: let
  required = pinName: field: attrs:
    if builtins.hasAttr field attrs
    then builtins.getAttr field attrs
    else throw "nix-pins: pin '${pinName}' is missing fetcher field '${field}'";
in {
  github = args: {
    _type = "github";
    inherit args;
  };

  evaluate = pinName: locked: declaration:
    if declaration._type or null == "github"
    then let
      args = declaration.args;
      owner = required pinName "owner" args;
      repo = required pinName "repo" args;
      packages = required pinName "packages" args;
      rev = locked.version or (throw "nix-pins: pin '${pinName}' is missing a locked version");
      fetcher = {github = {inherit owner repo rev;};};
      fetchArgs = removeAttrs args ["packages" "fetcherArgs"];
      src = pkgs.fetchFromGitHub (fetchArgs
        // (args.fetcherArgs or {})
        // {
          inherit owner repo rev;
          hash = locked.hash or fake;
        });
    in {
      check.github = "${owner}/${repo}";
      inherit fetcher packages src;
    }
    else throw "nix-pins: pin '${pinName}' uses unsupported fetcher '${declaration._type or "unknown"}'";
}
