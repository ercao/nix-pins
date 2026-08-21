{
  description = "Lock Nix package versions";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";

  outputs = {nixpkgs, ...}: let
    system = "aarch64-darwin";
    pkgs = import nixpkgs {inherit system;};
    package = pkgs.rustPlatform.buildRustPackage {
      pname = "nix-pins";
      version = "0.1.0";
      src = pkgs.lib.cleanSource ./.;
      cargoLock.lockFile = ./Cargo.lock;
      NIX_PINS_EVALUATOR = "${builtins.placeholder "out"}/share/nix-pins/nix/evaluator.nix";
      buildInputs = pkgs.lib.optionals pkgs.stdenv.hostPlatform.isDarwin [pkgs.libiconv];
      nativeCheckInputs = [pkgs.git pkgs.nix];
      preCheck = ''
        export NIX_REMOTE="local?root=$TMPDIR/nix-store"
      '';
      checkFlags = [
        "--test-threads=1"
        # capability-expansion 的 fixture 当前未跟踪，Nix cleanSource 不会收录。
        "--skip=probe::tests::orthogonal_git_and_url_fetchers_evaluate_through_the_public_nix_seam"
        # 真实网络 probe 不属于可复现的 Flake check。
        "--skip=probe::tests::probes_real_github_go_and_npm_derivations"
      ];
      postInstall = ''
        mkdir -p $out/share/nix-pins
        cp -R nix $out/share/nix-pins/
      '';
    };
  in {
    packages.${system}.default = package;
    apps.${system}.default = {
      type = "app";
      program = "${package}/bin/nix-pins";
    };
    checks.${system}.default = package;
  };
}
