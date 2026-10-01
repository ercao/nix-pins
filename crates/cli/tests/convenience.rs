use serde_json::{Value, json};
use std::process::Command;

fn eval(expr: &str) -> Result<Value, String> {
    let output = Command::new("nix")
        .args(["eval", "--json", "--impure", "--expr", expr])
        .output()
        .unwrap();

    if output.status.success() {
        serde_json::from_slice(&output.stdout).map_err(|error| error.to_string())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).into_owned())
    }
}

#[test]
fn convenience_pin_constructors_accept_string_targets() {
    let evaluator = concat!(env!("CARGO_MANIFEST_DIR"), "/../../nix/evaluator.nix");
    let expr = format!(
        r#"let
          pkgs = {{
            lib.fakeHash = "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
            fetchFromGitHub = args: args // {{ drvPath = "/nix/store/github-source.drv"; }};
            fetchgit = args: args // {{ drvPath = "/nix/store/git-source.drv"; }};
            applyPatches = args: args // {{ drvPath = "/nix/store/git-patched.drv"; }};
            buildGoModule = args: args // {{ goModules.drvPath = "/nix/store/git-go-modules.drv"; }};
          }};
          config = builtins.toFile "pins-config.nix" ''
            {{ pin }}: {{
              github-string = pin.github "acme/demo";
              github-attrs = pin.github {{ target = "acme/demo"; }};
              git-string = pin.git "https://example.com/string.git";
              git-attrs = pin.git {{
                target = "https://example.com/attrs.git";
                mode = "branch";
                branch = "main";
                rev = version: "refs/commits/''${{version}}";
                fetcherArgs.fetchSubmodules = true;
                postPatch = "echo patched";
                packages.default = pin.goModule {{ root = "."; }};
              }};
            }}
          '';
          cfg = import {evaluator} {{
            inherit pkgs config;
            pins = {{
              "github-string".version = "v1.2.3";
              "github-attrs".version = "v1.2.3";
              "git-string".version = "abc123";
              "git-attrs".version = "def456";
            }};
          }};
        in {{
          githubStringCheck = cfg."github-string".check;
          githubAttrsCheck = cfg."github-attrs".check;
          gitStringCheck = cfg."git-string".check;
          gitStringFetcher = cfg."git-string".sources.default.fetcher;
          gitStringSources = builtins.attrNames cfg."git-string".sources;
          gitAttrsCheck = cfg."git-attrs".check;
          gitAttrsFetcher = cfg."git-attrs".sources.default.fetcher;
          gitAttrsSrc = cfg."git-attrs".sources.default.src.drvPath;
          gitAttrsDerived = cfg."git-attrs".sources.default.derived;
        }}"#
    );

    let result = eval(&expr).unwrap();

    assert_eq!(result["githubStringCheck"], result["githubAttrsCheck"]);
    assert_eq!(result["githubStringCheck"], json!({"github": "acme/demo"}));
    assert_eq!(
        result["gitStringCheck"],
        json!({"git": {
            "url": "https://example.com/string.git",
            "mode": "head",
            "sort": "semver"
        }})
    );
    assert_eq!(
        result["gitStringFetcher"],
        json!({"git": {
            "url": "https://example.com/string.git",
            "rev": "abc123"
        }})
    );
    assert_eq!(result["gitStringSources"], json!(["default"]));
    assert_eq!(
        result["gitAttrsCheck"],
        json!({"git": {
            "url": "https://example.com/attrs.git",
            "mode": "branch",
            "branch": "main",
            "sort": "semver"
        }})
    );
    assert_eq!(
        result["gitAttrsFetcher"],
        json!({"git": {
            "url": "https://example.com/attrs.git",
            "rev": "refs/commits/def456",
            "fetchSubmodules": true
        }})
    );
    assert_eq!(result["gitAttrsSrc"], "/nix/store/git-patched.drv");
    assert_eq!(
        result["gitAttrsDerived"],
        json!({"vendorHash": "/nix/store/git-go-modules.drv"})
    );
}

#[test]
fn pin_git_rejects_url_as_a_target_alias() {
    let evaluator = concat!(env!("CARGO_MANIFEST_DIR"), "/../../nix/evaluator.nix");
    let expr = format!(
        r#"let
          pkgs.lib.fakeHash = "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
          config = builtins.toFile "pins-config.nix" ''
            {{ pin }}: {{ demo = pin.git {{ url = "https://example.com/demo.git"; }}; }}
          '';
          cfg = import {evaluator} {{ inherit pkgs config; }};
        in cfg.demo.check"#
    );

    let error = eval(&expr).unwrap_err();

    assert!(error.contains("pin 'demo'"), "{error}");
    assert!(error.contains("unknown field 'url'"), "{error}");
}

#[test]
fn convenience_pin_constructors_reject_other_input_types() {
    let evaluator = concat!(env!("CARGO_MANIFEST_DIR"), "/../../nix/evaluator.nix");

    for constructor in ["github", "git"] {
        let expr = format!(
            r#"let
              pkgs.lib.fakeHash = "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
              config = builtins.toFile "pins-config.nix" ''
                {{ pin }}: {{ demo = pin.{constructor} 42; }}
              '';
              cfg = import {evaluator} {{ inherit pkgs config; }};
            in cfg.demo.check"#
        );

        let error = eval(&expr).unwrap_err();

        assert!(
            error.contains(&format!("pin.{constructor} must receive a string or attribute set")),
            "{error}"
        );
    }
}
