# 统一 target 简写与旧字段，显式拒绝混用，避免 Checker 和 Fetcher 各取不同值。
{required}: {
  field = pinName: context: legacyField: args:
    if args ? target && builtins.hasAttr legacyField args
    then throw "nix-pins: pin '${pinName}' ${context} cannot combine fields 'target' and '${legacyField}'"
    else if args ? target
    then args.target
    else required pinName context legacyField args;

  github = pinName: context: args: let
    hasTarget = args ? target;
    hasLegacy = args ? owner || args ? repo;
    match =
      if hasTarget && builtins.isString args.target
      then builtins.match "([^/]+)/([^/]+)" args.target
      else null;
  in
    if hasTarget && hasLegacy
    then throw "nix-pins: pin '${pinName}' ${context} cannot combine field 'target' with 'owner' or 'repo'"
    else if hasTarget && match == null
    then throw "nix-pins: pin '${pinName}' ${context} field 'target' must be an 'owner/repo' string"
    else if hasTarget
    then {
      owner = builtins.elemAt match 0;
      repo = builtins.elemAt match 1;
    }
    else {
      owner = required pinName context "owner" args;
      repo = required pinName context "repo" args;
    };
}
