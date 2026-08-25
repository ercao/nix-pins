//! 通过声明式 evaluator 对用户 Nix 配置进行两阶段求值（ADR-0015、ADR-0016）。
//!
//! 阶段一以空锁定集只读 check；阶段二由工具注入版本和已知 source hash，
//! 读取 Fetcher、source 与各中间 FOD 的 drvPath。

use crate::checker::Checker;
use crate::nix::{self, FAKE};
use crate::pins::Fetcher;
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, serde::Deserialize)]
pub struct ProbeResult {
    pub sources: BTreeMap<String, SourceProbeResult>,
}

#[derive(Debug, serde::Deserialize)]
pub struct SourceProbeResult {
    pub src: String,
    #[serde(default)]
    pub patched: Option<String>,
    pub fetcher: Fetcher,
    /// Derived Hash 名 → 承载它的 Intermediate FOD drvPath（ADR-0011）。
    #[serde(default)]
    pub derived: BTreeMap<String, String>,
    /// Package 名 → Derived Hash 名 → Intermediate FOD drvPath（ADR-0020）。
    #[serde(default)]
    pub packages: BTreeMap<String, BTreeMap<String, String>>,
}

impl SourceProbeResult {
    pub fn package_derived(&self) -> BTreeMap<String, BTreeMap<String, String>> {
        if self.packages.is_empty() && !self.derived.is_empty() {
            BTreeMap::from([("default".into(), self.derived.clone())])
        } else {
            self.packages.clone()
        }
    }
}

/// 阶段一：列出各 Pin 的 checker 声明。pins 传空集。
pub fn probe_checks(config: &str) -> Result<BTreeMap<String, Checker>, nix::Error> {
    let config = config_path(config)?;
    let evaluator = evaluator_path()?;
    let expr = format!(
        "let pkgs = import <nixpkgs> {{}}; \
             cfg = import {evaluator} {{ inherit pkgs; config = {config}; pins = {{}}; }}; \
         in builtins.mapAttrs (n: p: p.check) cfg"
    );
    let value = nix::eval_json(&expr)?;
    serde_json::from_value(value).map_err(|error| nix::Error::Nix(error.to_string()))
}

/// 阶段二：注入已知版本和 source hash，取出 Fetcher 与各 FOD 的 drvPath。
pub fn probe_drvs(
    config: &str,
    versions: &BTreeMap<String, String>,
    hashes: &BTreeMap<String, BTreeMap<String, String>>,
) -> Result<BTreeMap<String, ProbeResult>, nix::Error> {
    let pins = versions
        .iter()
        .map(|(name, version)| {
            let sources: BTreeMap<String, serde_json::Value> = hashes
                .get(name)
                .map(|sources| {
                    sources
                        .iter()
                        .map(|(source, hash)| (source.clone(), serde_json::json!({"hash": hash})))
                        .collect()
                })
                .unwrap_or_default();
            (
                name.clone(),
                serde_json::json!({"version": version, "sources": sources}),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let pins = serde_json::to_string(&pins)
        .and_then(|pins| serde_json::to_string(&pins))
        .map_err(|error| nix::Error::Nix(error.to_string()))?;
    let config = config_path(config)?;
    let evaluator = evaluator_path()?;
    let expr = format!(
        "let
          pkgs = import <nixpkgs> {{}};
          pins = builtins.fromJSON {pins};
          cfg = import {evaluator} {{ inherit pkgs pins; config = {config}; fake = \"{FAKE}\"; }};
          # 保留旧 CLI seam 用来识别 Probe 阶段的标记：p.fetchSrc.drvPath
          all = builtins.listToAttrs (map (name: {{ inherit name; value = allPins.${{name}}; }}) (builtins.attrNames pins));
          allPins = cfg;
        in builtins.mapAttrs (_: pin: {{
          sources = builtins.mapAttrs (_: source: {{
            src = source.fetchSrc.drvPath;
            patched = if source.src.drvPath == source.fetchSrc.drvPath then null else source.src.drvPath;
            inherit (source) fetcher derived;
            packages = source.packageDerived;
          }}) pin.sources;
        }}) all"
    );
    let value = nix::eval_json(&expr)?;
    serde_json::from_value(value).map_err(|error| nix::Error::Nix(error.to_string()))
}

fn config_path(config: &str) -> Result<String, nix::Error> {
    let path = std::fs::canonicalize(config).map_err(|error| nix::Error::Nix(error.to_string()))?;
    serde_json::to_string(&path.to_string_lossy()).map_err(|error| nix::Error::Nix(error.to_string()))
}

fn evaluator_path() -> Result<String, nix::Error> {
    let source = concat!(env!("CARGO_MANIFEST_DIR"), "/nix/evaluator.nix");
    let path = option_env!("NIX_PINS_EVALUATOR")
        .filter(|path| Path::new(path).is_file())
        .unwrap_or(source);
    config_path(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pins::Fetcher;

    #[test]
    fn probes_real_github_go_and_npm_derivations() {
        let versions = BTreeMap::from([("curlie".into(), "v1.8.2".into()), ("sloc".into(), "0.3.2".into())]);
        let config = concat!(env!("CARGO_MANIFEST_DIR"), "/examples/example1/pins-config.nix");

        let results = probe_drvs(config, &versions, &BTreeMap::new()).unwrap();

        assert!(matches!(
            results["curlie"].sources["default"].fetcher,
            Fetcher::Github(_)
        ));
        assert!(results["curlie"].sources["default"].derived.contains_key("vendorHash"));
        assert!(results["sloc"].sources["default"].derived.contains_key("npmDepsHash"));
    }

    #[test]
    fn declarative_github_go_pin_evaluates_through_the_public_nix_seam() {
        let evaluator = concat!(env!("CARGO_MANIFEST_DIR"), "/nix/evaluator.nix");
        let config = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/declarative-go.nix");
        let expr = format!(
            r#"let
  pkgs = {{
    lib.fakeHash = "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
    fetchFromGitHub = args: args // {{ drvPath = "/nix/store/demo-source.drv"; }};
    applyPatches = args: args // {{ drvPath = "/nix/store/demo-patched.drv"; }};
    buildGoModule = args: args // {{ goModules.drvPath = "/nix/store/demo-go-modules.drv"; }};
  }};
  cfg = import {evaluator} {{
    inherit pkgs;
    config = {config};
    pins.demo = {{ version = "v1.2.3"; }};
  }};
  in {{
    inherit (cfg.demo) check;
    inherit (cfg.demo.sources.default) fetcher derived;
    fetchSrc = cfg.demo.sources.default.fetchSrc.drvPath;
    src = cfg.demo.sources.default.src.drvPath;
    patches = cfg.demo.sources.default.src.patches;
    postPatch = cfg.demo.sources.default.src.postPatch;
    packageSrc = cfg.demo.sources.default.packages.default.src.drvPath;
    package = cfg.demo.sources.default.packages.default.pname;
  root = cfg.demo.sources.default.packages.default.modRoot;
  ldflags = cfg.demo.sources.default.packages.default.ldflags;
}}"#
        );

        let result = nix::eval_json(&expr).unwrap();

        assert_eq!(result["check"], serde_json::json!({"github": "acme/demo"}));
        assert_eq!(
            result["fetcher"],
            serde_json::json!({"github": {"owner": "acme", "repo": "demo", "rev": "v1.2.3"}})
        );
        assert_eq!(result["fetchSrc"], "/nix/store/demo-source.drv");
        assert_eq!(result["src"], "/nix/store/demo-patched.drv");
        assert_eq!(result["patches"], serde_json::json!(["demo.patch"]));
        assert_eq!(result["postPatch"], "echo patched");
        assert_eq!(result["packageSrc"], "/nix/store/demo-patched.drv");
        assert_eq!(
            result["derived"],
            serde_json::json!({"vendorHash": "/nix/store/demo-go-modules.drv"})
        );
        assert_eq!(result["package"], "demo");
        assert_eq!(result["root"], "cmd/demo");
        assert_eq!(result["ldflags"], serde_json::json!(["-s"]));
    }

    #[test]
    fn probes_declarative_github_go_pins_through_the_cli_seam() {
        let config = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/declarative-go.nix");

        let checks = probe_checks(config).unwrap();
        let results = probe_drvs(
            config,
            &BTreeMap::from([("demo".into(), "v1.2.3".into())]),
            &BTreeMap::new(),
        )
        .unwrap();

        assert!(matches!(checks["demo"], crate::checker::Checker::Github(_)));
        assert!(matches!(results["demo"].sources["default"].fetcher, Fetcher::Github(_)));
        assert!(results["demo"].sources["default"].derived.contains_key("vendorHash"));
    }

    #[test]
    fn derived_probes_use_the_locked_source_hash() {
        let versions = BTreeMap::from([("cpa-manager-plus".into(), "v1.12.1".into())]);
        let hashes = BTreeMap::from([(
            "cpa-manager-plus".into(),
            BTreeMap::from([(
                "default".into(),
                "sha256-tq5F5NgKyahsYOmv5NDF1TMwc5OfTx18aCd2PyrvTNM=".into(),
            )]),
        )]);
        let config = concat!(env!("CARGO_MANIFEST_DIR"), "/examples/example2/pins-config.nix");

        let fake_source = probe_drvs(config, &versions, &BTreeMap::new()).unwrap();
        let locked_source = probe_drvs(config, &versions, &hashes).unwrap();

        assert_ne!(
            fake_source["cpa-manager-plus"].sources["default"].derived,
            locked_source["cpa-manager-plus"].sources["default"].derived
        );
        assert!(locked_source["cpa-manager-plus"].sources["default"]
            .derived
            .contains_key("vendorHash"));
        assert!(locked_source["cpa-manager-plus"].sources["default"]
            .derived
            .contains_key("npmDepsHash"));
        assert!(
            locked_source["cpa-manager-plus"].sources["default"].packages["manager-server"].contains_key("vendorHash")
        );
        assert!(locked_source["cpa-manager-plus"].sources["default"].packages["web"].contains_key("npmDepsHash"));
    }

    #[test]
    fn locked_evaluator_exposes_named_go_and_npm_packages() {
        let evaluator = concat!(env!("CARGO_MANIFEST_DIR"), "/nix/evaluator.nix");
        let config = concat!(env!("CARGO_MANIFEST_DIR"), "/examples/example2/pins-config.nix");
        let expr = format!(
            r#"let
  pkgs = import <nixpkgs> {{}};
  cfg = import {evaluator} {{
    inherit pkgs;
    config = {config};
    pins.cpa-manager-plus = {{
      version = "v1.12.1";
      hash = "sha256-tq5F5NgKyahsYOmv5NDF1TMwc5OfTx18aCd2PyrvTNM=";
      derived.vendorHash = "sha256-GBccl8V87u26dtrGpHR+rKqRBqX6lq1SBwfsPvj/+44=";
      derived.npmDepsHash = "sha256-yWVErql5SWOSbbw2DZUlXBJp7zqZngkc8uAC5ZfnjX0=";
    }};
  }};
in {{
  managerServer = cfg.cpa-manager-plus.sources.default.packages.manager-server.drvPath;
  web = cfg.cpa-manager-plus.sources.default.packages.web.drvPath;
}}"#
        );

        let result = nix::eval_json(&expr).unwrap();

        assert!(result["managerServer"]
            .as_str()
            .unwrap()
            .contains("cpa-manager-plus-manager-server-v1.12.1"));
        assert!(result["web"].as_str().unwrap().contains("cpa-manager-plus-web-v1.12.1"));
    }

    #[test]
    fn pins_file_entry_exposes_named_packages() {
        let packages = concat!(env!("CARGO_MANIFEST_DIR"), "/nix/packages.nix");
        let config = concat!(env!("CARGO_MANIFEST_DIR"), "/examples/example2/pins-config.nix");
        let pins_file = concat!(env!("CARGO_MANIFEST_DIR"), "/examples/example2/pins.json");
        let expr = format!(
            r#"let
  pkgs = import <nixpkgs> {{}};
  result = import {packages} {{
    inherit pkgs;
    config = {config};
    pinsFile = {pins_file};
  }};
  in {{
    src = result.cpa-manager-plus.sources.default.src.drvPath;
    managerServer = result.cpa-manager-plus.sources.default.packages.manager-server.drvPath;
  web = result.cpa-manager-plus.sources.default.packages.web.drvPath;
  pinHasNpmDeps = result.cpa-manager-plus ? npmDeps;
  sourceHasGoModules = result.cpa-manager-plus.sources.default ? goModules;
  packageHasNpmDeps = result.cpa-manager-plus.sources.default.packages.web ? npmDeps;
  packageHasGoModules = result.cpa-manager-plus.sources.default.packages.manager-server ? goModules;
}}"#
        );

        let result = nix::eval_json(&expr).unwrap();

        assert!(result["src"].as_str().unwrap().starts_with("/nix/store/"));
        assert!(result["managerServer"]
            .as_str()
            .unwrap()
            .contains("cpa-manager-plus-manager-server-v1.12.1"));
        assert!(result["web"].as_str().unwrap().contains("cpa-manager-plus-web-v1.12.1"));
        assert_eq!(result["pinHasNpmDeps"], false);
        assert_eq!(result["sourceHasGoModules"], false);
        assert_eq!(result["packageHasNpmDeps"], true);
        assert_eq!(result["packageHasGoModules"], true);
    }

    #[test]
    fn declarative_errors_name_the_pin_and_package() {
        let evaluator = concat!(env!("CARGO_MANIFEST_DIR"), "/nix/evaluator.nix");
        let config = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/invalid-declarations.nix");
        let cases = [
            ("cfg.missing-owner.check", ["pin 'missing-owner'", "field 'owner'"]),
            (
                "cfg.unsupported-fetcher.check",
                ["pin 'unsupported-fetcher'", "fetcher 'gitlab'"],
            ),
            (
                "cfg.unsupported-builder.sources.default.derived",
                ["pin 'unsupported-builder'", "package 'default'"],
            ),
            (
                "cfg.reserved-fetcher-arg.check",
                ["pin 'reserved-fetcher-arg'", "reserved field 'owner'"],
            ),
            (
                "cfg.non-string-url.sources.default.src.drvPath",
                ["pin 'non-string-url'", "map Version to a string"],
            ),
            (
                "cfg.invalid-github-target.check",
                ["pin 'invalid-github-target'", "'owner/repo' string"],
            ),
            (
                "cfg.conflicting-target.check",
                ["pin 'conflicting-target'", "fields 'target' and 'url'"],
            ),
        ];

        for (selection, expected) in cases {
            let expr = format!(
                r#"let
  pkgs = {{
    lib.fakeHash = "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
    fetchFromGitHub = args: args // {{ drvPath = "/nix/store/demo-source.drv"; }};
    fetchurl = args: args // {{ drvPath = "/nix/store/demo-url.drv"; }};
  }};
  cfg = import {evaluator} {{
    inherit pkgs;
    config = {config};
    pins = {{
    missing-owner.version = "v1";
    unsupported-fetcher.version = "v1";
    unsupported-builder.version = "v1";
    reserved-fetcher-arg.version = "v1";
    non-string-url.version = "v1";
    }};
  }};
in {selection}"#
            );
            let error = format!("{:?}", nix::eval_json(&expr).unwrap_err());

            for needle in expected {
                assert!(error.contains(needle), "{error}");
            }
        }
    }

    #[test]
    fn fetcher_mapping_is_validated_with_checks() {
        let config = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/non-string-fetcher.nix");
        let error = match probe_checks(config).unwrap_err() {
            nix::Error::NoGotLine(error) | nix::Error::Nix(error) => error,
        };

        assert!(error.contains("pin 'demo'"), "{error}");
        assert!(error.contains("map Version to a string"), "{error}");
    }

    #[test]
    fn orthogonal_fetchers_apply_default_and_custom_version_mappings() {
        let config = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/orthogonal-fetchers.nix");
        let checks = probe_checks(config).unwrap();
        assert!(matches!(checks["git-default"], crate::checker::Checker::Cmd(_)));
        assert!(matches!(checks["git-mapped"], crate::checker::Checker::Crate(_)));
        assert!(matches!(checks["url"], crate::checker::Checker::Pypi(_)));

        let versions = BTreeMap::from([
            ("git-default".into(), "v1.2.3".into()),
            ("git-mapped".into(), "2.0.0".into()),
            ("url".into(), "3.0.0".into()),
        ]);
        let results = probe_drvs(config, &versions, &BTreeMap::new()).unwrap();
        assert_eq!(
            serde_json::to_value(&results["git-default"].sources["default"].fetcher).unwrap(),
            serde_json::json!({"git": {
                "url": "https://example.com/default.git",
                "rev": "v1.2.3"
            }})
        );
        assert_eq!(
            serde_json::to_value(&results["git-mapped"].sources["default"].fetcher).unwrap(),
            serde_json::json!({"git": {
                "url": "https://example.com/mapped.git",
                "rev": "refs/tags/2.0.0"
            }})
        );
        assert_eq!(
            serde_json::to_value(&results["url"].sources["default"].fetcher).unwrap(),
            serde_json::json!({"url": {
                "url": "https://example.com/demo-3.0.0.tar.gz"
            }})
        );
    }

    #[test]
    fn orthogonal_git_and_url_fetchers_evaluate_through_the_public_nix_seam() {
        let config = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/declarative-orthogonal.nix");
        let checks = probe_checks(config).unwrap();
        assert!(matches!(checks["git-source"], crate::checker::Checker::Cmd(_)));
        assert!(matches!(checks["url-source"], crate::checker::Checker::Pypi(_)));
        assert!(matches!(checks["npm-source"], crate::checker::Checker::Npm(_)));
        assert!(matches!(
            checks["git-checker-url-source"],
            crate::checker::Checker::Git(_)
        ));
        assert!(format!("{:?}", checks["git-checker-url-source"]).contains("mode: Branch"));
        assert!(matches!(checks["crate-git-source"], crate::checker::Checker::Crate(_)));

        let versions = BTreeMap::from([
            ("git-source".into(), "v1.2.3".into()),
            ("url-source".into(), "2.0.0".into()),
            ("npm-source".into(), "3.0.0".into()),
            ("git-checker-url-source".into(), "main-sha".into()),
            ("crate-git-source".into(), "4.0.0".into()),
        ]);
        let results = probe_drvs(config, &versions, &BTreeMap::new()).unwrap();
        assert_eq!(
            serde_json::to_value(&results["git-source"].sources["default"].fetcher).unwrap(),
            serde_json::json!({
                "git": {
                    "url": "https://example.com/demo.git",
                    "rev": "refs/tags/v1.2.3"
                }
            })
        );
        assert_eq!(
            serde_json::to_value(&results["url-source"].sources["default"].fetcher).unwrap(),
            serde_json::json!({
                "url": {
                    "url": "https://example.com/demo-2.0.0.tar.gz"
                }
            })
        );
        assert_eq!(
            serde_json::to_value(&results["npm-source"].sources["default"].fetcher).unwrap(),
            serde_json::json!({
                "url": {
                    "url": "https://registry.npmjs.org/@scope/demo/-/demo-3.0.0.tgz"
                }
            })
        );
        assert_eq!(
            serde_json::to_value(&results["git-checker-url-source"].sources["default"].fetcher).unwrap(),
            serde_json::json!({
                "url": {
                    "url": "https://example.com/demo-main-sha.tar.gz"
                }
            })
        );
        assert_eq!(
            serde_json::to_value(&results["crate-git-source"].sources["default"].fetcher).unwrap(),
            serde_json::json!({
                "git": {
                    "url": "https://example.com/demo.git",
                    "rev": "refs/tags/4.0.0"
                }
            })
        );
    }

    #[test]
    fn multiple_same_type_packages_get_stable_derived_hash_names() {
        let evaluator = concat!(env!("CARGO_MANIFEST_DIR"), "/nix/evaluator.nix");
        let config = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/declarative-multi-go.nix");
        let expr = format!(
            r#"let
  pkgs = {{
    lib.fakeHash = "fake";
    fetchFromGitHub = args: args // {{ drvPath = "/nix/store/demo-source.drv"; }};
    buildGoModule = args: args // {{
      goModules.drvPath = "/nix/store/${{args.pname}}-go-modules.drv";
    }};
  }};
  cfg = import {evaluator} {{
    inherit pkgs;
    config = {config};
    pins.demo = {{
      version = "v1";
      sources.default.derived."api.vendorHash" = "api-hash";
      sources.default.derived."cli.vendorHash" = "cli-hash";
    }};
  }};
in {{
  inherit (cfg.demo.sources.default) derived packageDerived;
  apiHash = cfg.demo.sources.default.packages.api.vendorHash;
  cliHash = cfg.demo.sources.default.packages.cli.vendorHash;
}}"#
        );

        let result = nix::eval_json(&expr).unwrap();

        assert_eq!(
            result["derived"],
            serde_json::json!({
                "api.vendorHash": "/nix/store/demo-api-go-modules.drv",
                "cli.vendorHash": "/nix/store/demo-cli-go-modules.drv"
            })
        );
        assert_eq!(
            result["packageDerived"],
            serde_json::json!({
                "api": { "api.vendorHash": "/nix/store/demo-api-go-modules.drv" },
                "cli": { "cli.vendorHash": "/nix/store/demo-cli-go-modules.drv" }
            })
        );
        assert_eq!(result["apiHash"], "api-hash");
        assert_eq!(result["cliHash"], "cli-hash");
    }

    #[test]
    fn checker_and_fetcher_are_orthogonal_through_the_public_nix_seam() {
        let evaluator = concat!(env!("CARGO_MANIFEST_DIR"), "/nix/evaluator.nix");
        let expr = format!(
            r#"let
  pkgs = {{
    lib.fakeHash = "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
    fetchFromGitHub = args: args // {{ drvPath = "/nix/store/demo-source.drv"; }};
    applyPatches = args: args // {{ drvPath = "/nix/store/demo-patched.drv"; }};
  }};
  config = builtins.toFile "pins-config.nix" ''
    {{ pin }}: {{
      demo = pin.mk {{
        checker = pin.checker.github {{ target = "versions/demo"; }};
      fetcher = pin.fetcher.github {{
        target = "sources/demo";
        rev = version: "refs/tags/''${{version}}";
        fetcherArgs.fetchSubmodules = true;
      }};
      postPatch = "echo patched";
      }};
    }}
  '';
  cfg = import {evaluator} {{
    inherit pkgs config;
    pins.demo.version = "v1.2.3";
  }};
in {{
  inherit (cfg.demo) check;
  inherit (cfg.demo.sources.default) fetcher;
  drvPath = cfg.demo.sources.default.src.drvPath;
}}"#
        );

        let result = nix::eval_json(&expr).unwrap();

        assert_eq!(result["check"], serde_json::json!({"github": "versions/demo"}));
        assert_eq!(
            result["fetcher"],
            serde_json::json!({
                "github": {
                    "owner": "sources",
                    "repo": "demo",
                    "rev": "refs/tags/v1.2.3",
                    "fetchSubmodules": true
                }
            })
        );
        assert_eq!(result["drvPath"], "/nix/store/demo-patched.drv");
    }

    #[test]
    fn archive_fetchers_apply_version_mappings() {
        let config = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/archive-fetchers.nix");
        let checks = probe_checks(config).unwrap();
        assert!(matches!(checks["zip"], crate::checker::Checker::Cmd(_)));
        assert!(matches!(checks["huggingface"], crate::checker::Checker::Cmd(_)));

        let versions = BTreeMap::from([("zip".into(), "v1.2.3".into()), ("huggingface".into(), "2.0.0".into())]);
        let results = probe_drvs(config, &versions, &BTreeMap::new()).unwrap();

        assert_eq!(
            serde_json::to_value(&results["zip"].sources["default"].fetcher).unwrap(),
            serde_json::json!({"zip": {
                "url": "https://example.com/demo-v1.2.3.tar.gz",
                "stripRoot": false
            }})
        );
        assert_eq!(
            serde_json::to_value(&results["huggingface"].sources["default"].fetcher).unwrap(),
            serde_json::json!({"huggingface": {
                "repoId": "acme/demo",
                "rev": "refs/tags/2.0.0",
                "backend": "lfs",
                "repoType": "dataset"
            }})
        );
    }

    #[test]
    fn explicit_multi_source_pin_evaluates_through_the_public_nix_seam() {
        let config = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/multi-source.nix");
        let checks = probe_checks(config).unwrap();
        assert!(matches!(checks["release"], crate::checker::Checker::Cmd(_)));

        let versions = BTreeMap::from([("release".into(), "v1.2.3".into())]);
        let hashes = BTreeMap::from([(
            "release".into(),
            BTreeMap::from([("archive".into(), FAKE.into()), ("repository".into(), FAKE.into())]),
        )]);
        let results = probe_drvs(config, &versions, &hashes).unwrap();
        let release = &results["release"].sources;

        assert_eq!(
            serde_json::to_value(&release["archive"].fetcher).unwrap(),
            serde_json::json!({
                "url": {
                    "url": "https://example.com/demo-v1.2.3.tar.gz"
                }
            })
        );
        assert_eq!(
            serde_json::to_value(&release["repository"].fetcher).unwrap(),
            serde_json::json!({
                "git": {
                    "url": "https://example.com/demo.git",
                    "rev": "refs/tags/v1.2.3"
                }
            })
        );
        assert!(release["archive"].packages["web"].contains_key("npmDepsHash"));
        assert!(release["repository"].packages["api"].contains_key("vendorHash"));
    }
}
