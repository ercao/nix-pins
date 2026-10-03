# 同时打包 Rust CLI 与 Nix evaluator，使安装后的命令不依赖开发源码目录。
{
  description = "Lock Nix package versions";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";

  outputs = { self, nixpkgs, ... }: let
    systems = [
      "x86_64-linux"
      "aarch64-linux"
      "aarch64-darwin"
    ];
    forAllSystems = nixpkgs.lib.genAttrs systems;
    pkgsFor = system: import nixpkgs { inherit system; };
    mkPackage = system: let
      pkgs = pkgsFor system;
    in
      pkgs.rustPlatform.buildRustPackage {
        pname = "nix-pins";
        version = (fromTOML (builtins.readFile ./crates/cli/Cargo.toml)).package.version;
        # 文档站依赖与本地验证产物不参与 CLI 构建。
        src = pkgs.lib.fileset.toSource {
          root = ./.;
          fileset = pkgs.lib.fileset.unions [
            ./Cargo.toml
            ./Cargo.lock
            ./LICENSE
            ./crates
            ./nix
            ./examples
          ];
        };
        cargoLock.lockFile = ./Cargo.lock;
        cargoBuildFlags = [ "-p" "nix-pins" ];
        NIX_PINS_EVALUATOR = "${placeholder "out"}/share/nix-pins/nix/evaluator.nix";
        buildInputs = pkgs.lib.optionals pkgs.stdenv.hostPlatform.isDarwin [ pkgs.libiconv ];

        nativeCheckInputs = [
          pkgs.git
          pkgs.nix
        ];

        doCheck = false;
        preCheck = ''
          export NIX_REMOTE="local?root=$TMPDIR/nix-store"
          export NIX_PATH="nixpkgs=${pkgs.path}"
        '';
        checkFlags = [
          "--test-threads=1"
          # checkFlags 仅在显式开启 doCheck 后生效。
          "--skip=probe::tests::orthogonal_git_and_url_fetchers_evaluate_through_the_public_nix_seam"
          # 真实网络 probe 不属于可复现的 Flake check。
          "--skip=probe::tests::probes_real_github_go_and_npm_derivations"
        ];
        postInstall = ''
          mkdir -p $out/share/nix-pins
          cp -R nix $out/share/nix-pins/
        '';

        meta = {
          description = "Lock Nix package versions";
          homepage = "https://github.com/ercao/nix-pins";
          license = pkgs.lib.licenses.mit;
          mainProgram = "nix-pins";
          platforms = systems;
        };
      };
  in {
    packages = forAllSystems (system: {
      default = mkPackage system;
    });
    apps = forAllSystems (system: {
      default = {
        type = "app";
        program = "${self.packages.${system}.default}/bin/nix-pins";
      };
    });
    checks = forAllSystems (system: {
      default = self.packages.${system}.default;
    });

    devShells = forAllSystems (system: let
      pkgs = pkgsFor system;
    in {
      default = pkgs.mkShell {
        inputsFrom = [ self.packages.${system}.default ];
        packages = [
          pkgs.hk
          pkgs.rustfmt
        ];
      };
    });
  };
}
