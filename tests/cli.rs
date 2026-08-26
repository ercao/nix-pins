use serde_json::Value;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!("nix-pins-{name}-{nonce}"));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn write_executable(path: &Path, body: &str) {
    fs::write(path, body).unwrap();
    let mut permissions = fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).unwrap();
}

fn run(dir: &Path, args: &[&str]) -> std::process::Output {
    run_with_env(dir, args, &[])
}

fn run_with_env(dir: &Path, args: &[&str], env: &[(&str, &str)]) -> std::process::Output {
    let path = std::env::var_os("PATH").unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_nix-pins"));
    command
        .args(args)
        .current_dir(dir)
        .env("PATH", format!("{}:{}", dir.display(), path.to_string_lossy()));
    for (key, value) in env {
        command.env(key, value);
    }
    command.output().unwrap()
}

fn serial() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(())).lock().unwrap()
}

#[test]
fn update_creates_a_pins_file_through_the_cli_seam() {
    let _serial = serial();
    let dir = TempDir::new("update");
    fs::write(dir.0.join("pins-config.nix"), "{ pin }: {}\n").unwrap();
    write_executable(
        &dir.0.join("nix"),
        r#"#!/bin/sh
case "$1" in
  eval)
    case "$*" in
      *p.check*)
        printf '%s\n' '{"demo":{"cmd":"printf v1.2.3"}}'
        ;;
      *source.fetchSrc.drvPath*)
      printf '%s\n' '{"demo":{"sources":{"default":{"src":"/nix/store/demo-source.drv","patched":"/nix/store/demo-patched.drv","fetcher":{"github":{"owner":"acme","repo":"demo","rev":"v1.2.3"}},"derived":{}}}}}'
        ;;
      *)
        printf '%s\n' 'unexpected eval' >&2
        exit 2
        ;;
    esac
    ;;
  build)
    case "$*" in
      *demo-patched.drv*) exit 0 ;;
      *)
        printf '%s\n' 'error: hash mismatch' '  got: sha256-BlpIDik4hkU4c+KCyAmgUURIN362RDQID/qo6Ojp2Ek=' >&2
        exit 1
        ;;
    esac
    ;;
esac
"#,
    );

    let output = run(&dir.0, &["update"]);

    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(String::from_utf8_lossy(&output.stderr).contains("Writing pins.json done"));
    let pins: Value = serde_json::from_slice(&fs::read(dir.0.join("pins.json")).unwrap()).unwrap();
    assert_eq!(pins["schemaVersion"], 2);
    assert_eq!(pins["pins"]["demo"]["version"], "v1.2.3");
    assert_eq!(
        pins["pins"]["demo"]["sources"]["default"]["hash"],
        "sha256-BlpIDik4hkU4c+KCyAmgUURIN362RDQID/qo6Ojp2Ek="
    );
    assert_eq!(
        pins["pins"]["demo"]["sources"]["default"]["fetcher"]["github"],
        serde_json::json!({"owner": "acme", "repo": "demo", "rev": "v1.2.3"})
    );
    assert_eq!(
        pins["pins"]["demo"]["sources"]["default"]["fingerprints"]["hash"],
        "/nix/store/demo-source.drv"
    );
    assert!(pins["pins"]["demo"]["sources"]["default"].get("derived").is_none());
}

#[test]
fn patch_failure_without_packages_rolls_back_the_pin() {
    let _serial = serial();
    let dir = TempDir::new("patch-failure");
    fs::write(dir.0.join("pins-config.nix"), "{ pin }: {}\n").unwrap();
    write_executable(
        &dir.0.join("nix"),
        r#"#!/bin/sh
case "$1" in
  eval)
    case "$*" in
      *p.check*) printf '%s\n' '{"demo":{"cmd":"printf v1"}}' ;;
      *source.fetchSrc.drvPath*) printf '%s\n' '{"demo":{"sources":{"default":{"src":"/nix/store/demo-source.drv","patched":"/nix/store/demo-patched.drv","fetcher":{"url":{"url":"https://example.invalid/demo"}},"derived":{}}}}}' ;;
    esac
    ;;
  build)
    case "$*" in
      *demo-patched.drv*)
        printf '%s\n' 'patch does not apply' >&2
        exit 1
        ;;
      *)
        printf '%s\n' 'got: sha256-source' >&2
        exit 1
        ;;
    esac
    ;;
esac
"#,
    );

    let output = run(&dir.0, &["update"]);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(!output.status.success());
    let pins: Value = serde_json::from_slice(&fs::read(dir.0.join("pins.json")).unwrap()).unwrap();
    assert!(pins["pins"].get("demo").is_none());
    assert!(pins["failures"]["demo"].as_str().unwrap().contains("Applying patches"));
    assert!(stderr.contains("⚠ demo"), "{stderr}");
    assert!(stderr.contains("Source default/Applying patches"), "{stderr}");
}

#[test]
fn schema_v1_is_rejected_by_the_cli_and_public_reader() {
    let _serial = serial();
    let dir = TempDir::new("schema-v1");
    fs::write(dir.0.join("pins-config.nix"), "{ pin }: {}\n").unwrap();
    fs::write(
        dir.0.join("pins.json"),
        b"{\n  \"schemaVersion\": 1,\n  \"pins\": {}\n}\n",
    )
    .unwrap();

    let status = run(&dir.0, &["status"]);
    assert_eq!(status.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&status.stderr).contains("unsupported Pins File schemaVersion 1; expected 2"));

    let reader = Path::new(env!("CARGO_MANIFEST_DIR")).join("pins.nix");
    let pins_path = dir.0.join("pins.json");
    let expression = format!(
        r#"let
          pins = import {reader} {{
            file = {pins};
            pkgs = {{}};
          }};
        in builtins.deepSeq pins true"#,
        reader = reader.display(),
        pins = pins_path.display(),
    );
    let output = Command::new("nix")
        .args(["eval", "--impure", "--json", "--expr", &expression])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("unsupported Pins File schemaVersion 1; expected 2"));
}

#[test]
fn multi_source_pin_updates_atomically_with_one_checker_result() {
    let _serial = serial();
    let dir = TempDir::new("multi-source-update");
    fs::write(dir.0.join("pins-config.nix"), "{ pin }: {}\n").unwrap();
    write_executable(
        &dir.0.join("check-version"),
        r#"#!/bin/sh
count=0
if test -f checker-count; then count=$(cat checker-count); fi
count=$((count + 1))
printf '%s' "$count" > checker-count
printf '%s\n' v1.2.3
"#,
    );
    write_executable(
        &dir.0.join("nix"),
        r#"#!/bin/sh
case "$1" in
  eval)
    case "$*" in
      *p.check*)
        printf '%s\n' '{"release":{"cmd":"./check-version"}}'
        ;;
      *source.fetchSrc.drvPath*)
      printf '%s\n' '{"release":{"sources":{"archive":{"src":"/nix/store/archive.drv","fetcher":{"url":{"url":"https://example.com/demo-v1.2.3.tar.gz"}},"derived":{"npmDepsHash":"/nix/store/npm-deps.drv"},"packages":{"web":{"npmDepsHash":"/nix/store/npm-deps.drv"}}},"repository":{"src":"/nix/store/repository.drv","patched":"/nix/store/repository-patched.drv","fetcher":{"git":{"url":"https://example.com/demo.git","rev":"refs/tags/v1.2.3"}},"derived":{"vendorHash":"/nix/store/go-modules.drv"},"packages":{"api":{"vendorHash":"/nix/store/go-modules.drv"}}}}}}'
        ;;
      *)
        printf '%s\n' 'unexpected eval' >&2
        exit 2
        ;;
    esac
    ;;
  build)
    case "$*" in
      *repository-patched.drv*) exit 0 ;;
      *)
        printf '%s\n' 'error: hash mismatch' 'got: sha256-BlpIDik4hkU4c+KCyAmgUURIN362RDQID/qo6Ojp2Ek=' >&2
        exit 1
        ;;
    esac
    ;;
esac
"#,
    );

    let source_selection = run(&dir.0, &["update", "archive"]);
    assert_eq!(source_selection.status.code(), Some(1));
    let selection_stderr = String::from_utf8_lossy(&source_selection.stderr);
    assert!(selection_stderr.contains("unknown pin name"), "{selection_stderr}");
    assert!(!dir.0.join("checker-count").exists());

    let output = run(&dir.0, &["update"]);

    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("release — → v1.2.3"), "{stderr}");
    assert_eq!(fs::read_to_string(dir.0.join("checker-count")).unwrap(), "1");
    let pins: Value = serde_json::from_slice(&fs::read(dir.0.join("pins.json")).unwrap()).unwrap();
    assert_eq!(pins["schemaVersion"], 2);
    assert_eq!(pins["pins"]["release"]["version"], "v1.2.3");
    assert_eq!(
        pins["pins"]["release"]["sources"]["archive"]["fetcher"]["url"]["url"],
        "https://example.com/demo-v1.2.3.tar.gz"
    );
    assert_eq!(
        pins["pins"]["release"]["sources"]["repository"]["fetcher"]["git"]["rev"],
        "refs/tags/v1.2.3"
    );
    assert_eq!(
        pins["pins"]["release"]["sources"]["archive"]["derived"]["npmDepsHash"],
        "sha256-BlpIDik4hkU4c+KCyAmgUURIN362RDQID/qo6Ojp2Ek="
    );
    assert_eq!(
        pins["pins"]["release"]["sources"]["repository"]["derived"]["vendorHash"],
        "sha256-BlpIDik4hkU4c+KCyAmgUURIN362RDQID/qo6Ojp2Ek="
    );

    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let reader = root.join("pins.nix");
    let config = root.join("tests/fixtures/multi-source.nix");
    let pins_path = dir.0.join("pins.json");
    let expression = format!(
        r#"let
  pins = import {reader} {{
    file = {pins};
    config = {config};
    pkgs = {{
      lib.fakeHash = "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
      fetchgit = args: args // {{ drvPath = "/nix/store/repository.drv"; }};
      fetchurl = args: args // {{ drvPath = "/nix/store/archive.drv"; }};
      applyPatches = args: args // {{ drvPath = "/nix/store/repository-patched.drv"; }};
      buildGoModule = args: args // {{ goModules.drvPath = "/nix/store/go-modules.drv"; }};
      buildNpmPackage = args: args // {{ npmDeps.drvPath = "/nix/store/npm-deps.drv"; }};
    }};
  }};
in {{
  inherit (pins.release) version;
  archive = pins.release.sources.archive.src.url;
  repository = pins.release.sources.repository.fetcher.git.rev;
  repositoryPatched = pins.release.sources.repository.src.drvPath;
  api = {{
    inherit (pins.release.sources.repository.packages.api) vendorHash modRoot version;
    src = pins.release.sources.repository.packages.api.src.drvPath;
  }};
  web = {{
    inherit (pins.release.sources.archive.packages.web) npmDepsHash npmRoot version;
    src = pins.release.sources.archive.packages.web.src.drvPath;
  }};
  sourceNames = builtins.attrNames pins.release.sources;
  promoted = pins.release ? src;
}}"#,
        reader = reader.display(),
        config = config.display(),
        pins = pins_path.display(),
    );
    let output = Command::new("nix")
        .args(["eval", "--impure", "--json", "--expr", &expression])
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["version"], "v1.2.3");
    assert_eq!(result["archive"], "https://example.com/demo-v1.2.3.tar.gz");
    assert_eq!(result["repository"], "refs/tags/v1.2.3");
    assert_eq!(result["repositoryPatched"], "/nix/store/repository-patched.drv");
    assert_eq!(
        result["api"]["vendorHash"],
        "sha256-BlpIDik4hkU4c+KCyAmgUURIN362RDQID/qo6Ojp2Ek="
    );
    assert_eq!(result["api"]["modRoot"], "cmd/api");
    assert_eq!(result["api"]["version"], "v1.2.3");
    assert_eq!(result["api"]["src"], "/nix/store/repository-patched.drv");
    assert_eq!(
        result["web"]["npmDepsHash"],
        "sha256-BlpIDik4hkU4c+KCyAmgUURIN362RDQID/qo6Ojp2Ek="
    );
    assert_eq!(result["web"]["npmRoot"], "web");
    assert_eq!(result["web"]["version"], "v1.2.3");
    assert_eq!(result["web"]["src"], "/nix/store/archive.drv");
    assert_eq!(result["sourceNames"], serde_json::json!(["archive", "repository"]));
    assert_eq!(result["promoted"], false);
}

#[test]
fn multi_source_failure_rolls_back_only_its_pin() {
    let _serial = serial();
    let dir = TempDir::new("multi-source-failure");
    fs::write(dir.0.join("pins-config.nix"), "{ pin }: {}\n").unwrap();
    let original_release = serde_json::json!({
        "version": "v1",
        "sources": {
            "bad": {
                "fetcher": {"url": {"url": "https://old.invalid/bad"}},
                "hash": "sha256-old-bad",
                "fingerprints": {"hash": "/nix/store/old-bad.drv"}
            },
            "good": {
                "fetcher": {"url": {"url": "https://old.invalid/good"}},
                "hash": "sha256-old-good",
                "fingerprints": {"hash": "/nix/store/old-good.drv"}
            }
        }
    });
    fs::write(
        dir.0.join("pins.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "schemaVersion": 2,
            "pins": {
                "release": original_release,
                "independent": {
                    "version": "v1",
                    "sources": {"default": {
                        "fetcher": {"url": {"url": "https://old.invalid/independent"}},
                        "hash": "sha256-old-independent",
                        "fingerprints": {"hash": "/nix/store/old-independent.drv"}
                    }}
                }
            }
        }))
        .unwrap(),
    )
    .unwrap();
    write_executable(
        &dir.0.join("nix"),
        r#"#!/bin/sh
case "$1" in
  eval)
    case "$*" in
      *p.check*)
        printf '%s\n' '{"independent":{"cmd":"printf v2"},"release":{"cmd":"printf v2"}}'
        ;;
      *source.fetchSrc.drvPath*)
        printf '%s\n' '{"independent":{"sources":{"default":{"src":"/nix/store/new-independent.drv","fetcher":{"url":{"url":"https://new.invalid/independent"}},"derived":{}}}},"release":{"sources":{"bad":{"src":"/nix/store/new-bad.drv","fetcher":{"url":{"url":"https://new.invalid/bad"}},"derived":{}},"good":{"src":"/nix/store/new-good.drv","fetcher":{"url":{"url":"https://new.invalid/good"}},"derived":{}}}}}'
        ;;
      *) exit 2 ;;
    esac
    ;;
  build)
    case "$*" in
      *new-bad.drv*)
        printf '%s\n' 'bad source failed' >&2
        exit 1
        ;;
      *new-good.drv*)
        printf '%s\n' 'good source also failed' >&2
        exit 1
        ;;
      *)
        printf '%s\n' 'error: hash mismatch' 'got: sha256-BlpIDik4hkU4c+KCyAmgUURIN362RDQID/qo6Ojp2Ek=' >&2
        exit 1
        ;;
    esac
    ;;
esac
"#,
    );

    let output = run(&dir.0, &["update"]);

    assert!(!output.status.success());
    let pins: Value = serde_json::from_slice(&fs::read(dir.0.join("pins.json")).unwrap()).unwrap();
    assert_eq!(pins["pins"]["release"], original_release);
    assert_eq!(pins["pins"]["independent"]["version"], "v2");
    assert_eq!(
        pins["pins"]["independent"]["sources"]["default"]["hash"],
        "sha256-BlpIDik4hkU4c+KCyAmgUURIN362RDQID/qo6Ojp2Ek="
    );
    assert_eq!(pins["failures"].as_object().unwrap().len(), 1);
    let failure = pins["failures"]["release"].as_str().unwrap();
    let bad = failure.find("Source 'bad' Hashing source").unwrap();
    let good = failure.find("Source 'good' Hashing source").unwrap();
    assert!(bad < good, "{failure}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("⚠ release"), "{stderr}");
    assert!(
        stderr.contains("Source bad/Hashing source, Source good/Hashing source"),
        "{stderr}"
    );
}

#[test]
fn invalid_source_configuration_fails_before_checker_and_preserves_pins_file() {
    let _serial = serial();
    let dir = TempDir::new("invalid-multi-source");
    fs::write(
        dir.0.join("pins-config.nix"),
        r#"{ pin }: {
  release = pin.mk {
    checker = pin.checker.cmd "touch checker-ran; printf v2";
    sources.bad = {
      checker = pin.checker.cmd "printf ignored";
      fetcher = pin.fetcher.url { target = "https://example.invalid/demo"; };
    };
  };
}
"#,
    )
    .unwrap();
    let original = br#"{
  "schemaVersion": 2,
  "pins": {}
}
"#;
    fs::write(dir.0.join("pins.json"), original).unwrap();

    let output = run(&dir.0, &["update"]);

    assert!(!output.status.success());
    assert!(!dir.0.join("checker-ran").exists());
    assert_eq!(fs::read(dir.0.join("pins.json")).unwrap(), original);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("source 'bad'"), "{stderr}");
    assert!(stderr.contains("unknown field 'checker'"), "{stderr}");
}

#[test]
fn explicit_sources_take_precedence_over_top_level_source_fields() {
    let _serial = serial();
    let dir = TempDir::new("multi-source-precedence");
    fs::write(
        dir.0.join("pins-config.nix"),
        r#"{ pin }: {
  release = pin.mk {
    checker = pin.checker.cmd "printf checker-failed >&2; exit 1";
    fetcher = "ignored-invalid-top-level-fetcher";
    packages.ignored = "ignored-invalid-top-level-package";
    sources.archive.fetcher = pin.fetcher.url {
      target = version: "https://example.invalid/demo-${version}";
    };
  };
}
"#,
    )
    .unwrap();

    let output = run(&dir.0, &["update"]);

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("checker-failed"), "{stderr}");
    assert!(!stderr.contains("map Version"), "{stderr}");
    assert!(!stderr.contains("unsupported builder"), "{stderr}");
}

#[test]
fn empty_sources_and_missing_fetcher_fail_before_checker() {
    let _serial = serial();
    for (name, declaration, expected) in [
        ("empty", "sources = {};", "must not be empty"),
        ("missing-fetcher", "sources.archive = {};", "missing field 'fetcher'"),
    ] {
        let dir = TempDir::new(name);
        fs::write(
            dir.0.join("pins-config.nix"),
            format!(
                r#"{{ pin }}: {{
  release = pin.mk {{
    checker = pin.checker.cmd "touch checker-ran; printf v2";
    {declaration}
  }};
}}
"#
            ),
        )
        .unwrap();
        let original = b"{\n  \"schemaVersion\": 2,\n  \"pins\": {}\n}\n";
        fs::write(dir.0.join("pins.json"), original).unwrap();

        let output = run(&dir.0, &["update"]);

        assert!(!output.status.success(), "{name}");
        assert!(!dir.0.join("checker-ran").exists(), "{name}");
        assert_eq!(fs::read(dir.0.join("pins.json")).unwrap(), original, "{name}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains(expected), "{name}: {stderr}");
    }
}

#[test]
fn orthogonal_public_dsl_updates_the_pins_file() {
    let _serial = serial();
    let dir = TempDir::new("orthogonal-update");
    fs::write(
        dir.0.join("pins-config.nix"),
        r#"{ pin }: {
  demo = pin.mk {
    checker = pin.checker.cmd "printf v1.2.3";
    fetcher = pin.fetcher.url {
      target = version: "https://example.com/demo-${version}.tar.gz";
    };
  };
}
"#,
    )
    .unwrap();
    let nix = Command::new("sh").args(["-c", "command -v nix"]).output().unwrap();
    assert!(nix.status.success());
    let nix = String::from_utf8(nix.stdout).unwrap();
    write_executable(
        &dir.0.join("nix"),
        &format!(
            r#"#!/bin/sh
case "$1" in
  eval)
    exec {} "$@"
    ;;
  build)
    printf '%s\n' 'error: hash mismatch' '  got: sha256-BlpIDik4hkU4c+KCyAmgUURIN362RDQID/qo6Ojp2Ek=' >&2
    exit 1
    ;;
esac
"#,
            nix.trim()
        ),
    );

    let output = run(&dir.0, &["update"]);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let pins: Value = serde_json::from_slice(&fs::read(dir.0.join("pins.json")).unwrap()).unwrap();
    assert_eq!(pins["pins"]["demo"]["version"], "v1.2.3");
    assert_eq!(
        pins["pins"]["demo"]["sources"]["default"]["hash"],
        "sha256-BlpIDik4hkU4c+KCyAmgUURIN362RDQID/qo6Ojp2Ek="
    );
    assert_eq!(
        pins["pins"]["demo"]["sources"]["default"]["fetcher"],
        serde_json::json!({
            "url": {
                "url": "https://example.com/demo-v1.2.3.tar.gz"
            }
        })
    );
    assert!(pins["pins"]["demo"]["sources"]["default"]["fingerprints"]["hash"]
        .as_str()
        .is_some_and(|src| src.starts_with("/nix/store/")));
}

#[test]
fn update_keeps_failed_pins_and_writes_successful_ones() {
    let _serial = serial();
    let dir = TempDir::new("partial-failure");
    fs::write(dir.0.join("pins-config.nix"), "{ pin }: {}\n").unwrap();
    fs::write(
        dir.0.join("pins.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "schemaVersion": 2,
            "pins": {
                "bad": {
                    "version": "v1",
                    "sources": {"default": {
                        "fetcher": {"url": {"url": "https://old.invalid/bad"}},
                        "hash": "sha256-old-bad",
                        "fingerprints": {"hash": "/nix/store/old-bad.drv"}
                    }}
                },
                "good": {
                    "version": "v1",
                    "sources": {"default": {
                        "fetcher": {"url": {"url": "https://old.invalid/good"}},
                        "hash": "sha256-old-good",
                        "fingerprints": {"hash": "/nix/store/old-good.drv"}
                    }}
                }
            }
        }))
        .unwrap(),
    )
    .unwrap();
    write_executable(
        &dir.0.join("nix"),
        r#"#!/bin/sh
case "$1" in
  eval)
    case "$*" in
      *p.check*)
        printf '%s\n' '{"bad":{"cmd":"printf checker-failed >&2; exit 1"},"good":{"cmd":"printf v2"}}'
        ;;
      *source.fetchSrc.drvPath*)
      printf '%s\n' '{"good":{"sources":{"default":{"src":"/nix/store/new-good.drv","fetcher":{"url":{"url":"https://new.invalid/good"}},"derived":{}}}}}'
        ;;
    esac
    ;;
  build)
    printf '%s\n' '  got: sha256-new-good' >&2
    exit 1
    ;;
esac
"#,
    );

    let output = run(&dir.0, &["update"]);

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains("\u{1b}"), "{stderr}");
    assert!(!stderr.contains("├─"), "{stderr}");
    assert!(!stderr.contains("└─"), "{stderr}");
    assert!(!stderr.contains("Pin Progress"), "{stderr}");
    assert!(stderr.contains("Processed 2 pins; 1 failed"), "{stderr}");
    assert!(stderr.contains("Failures:"), "{stderr}");
    assert!(
        stderr.find("Failures:") < stderr.find("Processed 2 pins; 1 failed"),
        "{stderr}"
    );
    assert!(stderr.contains("bad"), "{stderr}");
    assert!(stderr.contains("checker-failed"), "{stderr}");
    let pins: Value = serde_json::from_slice(&fs::read(dir.0.join("pins.json")).unwrap()).unwrap();
    assert_eq!(pins["pins"]["bad"]["version"], "v1");
    assert_eq!(pins["pins"]["bad"]["sources"]["default"]["hash"], "sha256-old-bad");
    assert_eq!(pins["pins"]["good"]["version"], "v2");
    assert_eq!(pins["pins"]["good"]["sources"]["default"]["hash"], "sha256-new-good");
}

#[test]
fn update_preserves_the_full_build_output_when_no_hash_is_reported() {
    let _serial = serial();
    let dir = TempDir::new("no-got");
    fs::write(dir.0.join("pins-config.nix"), "{ pin }: {}\n").unwrap();
    write_executable(
        &dir.0.join("nix"),
        r#"#!/bin/sh
case "$1" in
  eval)
    case "$*" in
      *p.check*)
        printf '%s\n' '{"demo":{"cmd":"printf v1"}}'
        ;;
      *source.fetchSrc.drvPath*)
      printf '%s\n' '{"demo":{"sources":{"default":{"src":"/nix/store/demo-source.drv","fetcher":{"url":{"url":"https://example.invalid/demo"}},"derived":{}}}}}'
        ;;
    esac
    ;;
  build)
    printf '%s\n' 'stdout diagnostic is preserved'
    printf '%s\n' 'download failed before hash comparison' 'the complete diagnostic is here' >&2
    exit 0
    ;;
esac
"#,
    );

    let output = run(&dir.0, &["update"]);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(!output.status.success());
    assert!(stderr.contains("stdout diagnostic is preserved"));
    assert!(stderr.contains("download failed before hash comparison"));
    assert!(stderr.contains("the complete diagnostic is here"));
}

#[test]
fn status_reports_versions_and_last_failures_without_running_checkers() {
    let _serial = serial();
    let dir = TempDir::new("status");
    fs::write(dir.0.join("pins-config.nix"), "{ pin }: {}\n").unwrap();
    fs::write(
        dir.0.join("pins.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "schemaVersion": 2,
            "pins": {
                "bad": {
                    "version": "v1",
                    "sources": {"default": {
                        "fetcher": {"url": {"url": "https://example.invalid/bad"}},
                        "hash": "sha256-bad"
                    }}
                },
                "good": {
                    "version": "v2",
                    "sources": {"default": {
                        "fetcher": {"url": {"url": "https://example.invalid/good"}},
                        "hash": "sha256-good"
                    }}
                }
            },
            "failures": {"bad": "rate limit exhausted"}
        }))
        .unwrap(),
    )
    .unwrap();
    write_executable(
        &dir.0.join("nix"),
        r#"#!/bin/sh
case "$1" in
  eval)
    printf '%s\n' '{"bad":{"cmd":"exit 99"},"good":{"cmd":"exit 99"}}'
    ;;
  *)
    printf '%s\n' 'status performed network or build work' >&2
    exit 99
    ;;
esac
"#,
    );

    let output = run(&dir.0, &["status"]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(stdout.contains("bad v1"));
    assert!(stdout.contains("rate limit exhausted"));
    assert!(stdout.contains("good v2"));
}

#[test]
fn update_reuses_hashes_when_the_fake_hash_fingerprint_is_unchanged() {
    let _serial = serial();
    let dir = TempDir::new("fingerprint");
    fs::write(dir.0.join("pins-config.nix"), "{ pin }: {}\n").unwrap();
    fs::write(
        dir.0.join("pins.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "schemaVersion": 2,
            "pins": {
                "demo": {
                    "version": "v1",
                    "sources": {"default": {
                        "fetcher": {"url": {"url": "https://example.invalid/demo"}},
                        "hash": "sha256-existing",
                        "fingerprints": {"hash": "/nix/store/unchanged-source.drv"}
                    }}
                }
            }
        }))
        .unwrap(),
    )
    .unwrap();
    write_executable(
        &dir.0.join("nix"),
        r#"#!/bin/sh
case "$1" in
  eval)
    case "$*" in
      *p.check*)
        printf '%s\n' '{"demo":{"cmd":"printf v1"}}'
        ;;
      *source.fetchSrc.drvPath*)
      printf '%s\n' '{"demo":{"sources":{"default":{"src":"/nix/store/unchanged-source.drv","fetcher":{"url":{"url":"https://example.invalid/demo"}},"derived":{}}}}}'
        ;;
    esac
    ;;
  build)
    printf '%s\n' 'hash build should have been skipped' >&2
    exit 99
    ;;
esac
"#,
    );

    let output = run(&dir.0, &["update"]);

    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let pins: Value = serde_json::from_slice(&fs::read(dir.0.join("pins.json")).unwrap()).unwrap();
    assert_eq!(pins["pins"]["demo"]["sources"]["default"]["hash"], "sha256-existing");
}

#[test]
fn update_writes_and_reuses_derived_hashes() {
    let _serial = serial();
    let dir = TempDir::new("derived");
    fs::write(dir.0.join("pins-config.nix"), "{ pin }: {}\n").unwrap();
    let nix = dir.0.join("nix");
    write_executable(
        &nix,
        r#"#!/bin/sh
case "$1" in
  eval)
    case "$*" in
      *p.check*)
        printf '%s\n' '{"demo":{"cmd":"printf v1"}}'
        ;;
      *source.fetchSrc.drvPath*)
        case "$*" in
          *sha256-source*)
        printf '%s\n' '{"demo":{"sources":{"default":{"src":"/nix/store/demo-source.drv","fetcher":{"github":{"owner":"acme","repo":"demo","rev":"v1"}},"derived":{"vendorHash":"/nix/store/demo-go-modules.drv","npmDepsHash":"/nix/store/demo-npm-deps.drv"}}}}}'
            ;;
          *)
        printf '%s\n' '{"demo":{"sources":{"default":{"src":"/nix/store/demo-source.drv","fetcher":{"github":{"owner":"acme","repo":"demo","rev":"v1"}},"derived":{"vendorHash":"/nix/store/source-mismatch.drv","npmDepsHash":"/nix/store/source-mismatch.drv"}}}}}'
            ;;
        esac
        ;;
    esac
    ;;
  build)
    case "$*" in
      *npm-deps*) printf '%s\n' '  got: sha256-npm' >&2 ;;
      *go-modules*) printf '%s\n' '  got: sha256-vendor' >&2 ;;
      *source-mismatch*) printf '%s\n' '  got: sha256-wrong-derived' >&2 ;;
      *) printf '%s\n' '  got: sha256-source' >&2 ;;
    esac
    exit 1
    ;;
esac
"#,
    );

    let first = run(&dir.0, &["update"]);
    assert!(first.status.success(), "{}", String::from_utf8_lossy(&first.stderr));
    let pins: Value = serde_json::from_slice(&fs::read(dir.0.join("pins.json")).unwrap()).unwrap();
    assert_eq!(pins["pins"]["demo"]["sources"]["default"]["hash"], "sha256-source");
    assert_eq!(
        pins["pins"]["demo"]["sources"]["default"]["derived"]["vendorHash"],
        "sha256-vendor"
    );
    assert_eq!(
        pins["pins"]["demo"]["sources"]["default"]["derived"]["npmDepsHash"],
        "sha256-npm"
    );
    assert_eq!(
        pins["pins"]["demo"]["sources"]["default"]["fingerprints"]["vendorHash"],
        "/nix/store/demo-go-modules.drv"
    );
    assert_eq!(
        pins["pins"]["demo"]["sources"]["default"]["fingerprints"]["npmDepsHash"],
        "/nix/store/demo-npm-deps.drv"
    );

    write_executable(
        &nix,
        r#"#!/bin/sh
case "$1" in
  eval)
    case "$*" in
      *p.check*) printf '%s\n' '{"demo":{"cmd":"printf v1"}}' ;;
      *source.fetchSrc.drvPath*) printf '%s\n' '{"demo":{"sources":{"default":{"src":"/nix/store/demo-source.drv","fetcher":{"github":{"owner":"acme","repo":"demo","rev":"v1"}},"derived":{"vendorHash":"/nix/store/demo-go-modules.drv","npmDepsHash":"/nix/store/demo-npm-deps.drv"}}}}}' ;;
    esac
    ;;
  build)
    printf '%s\n' 'derived build should have been skipped' >&2
    exit 99
    ;;
esac
"#,
    );
    let second = run(&dir.0, &["update"]);
    let stderr = String::from_utf8_lossy(&second.stderr);
    assert!(second.status.success(), "{stderr}");
    assert!(stderr.contains("✔ demo v1"), "{stderr}");
}

#[test]
fn package_failures_are_aggregated_in_stable_order() {
    let _serial = serial();
    let dir = TempDir::new("package-failures");
    fs::write(dir.0.join("pins-config.nix"), "{ pin }: {}\n").unwrap();
    let original = serde_json::json!({
        "version": "v1",
        "sources": {"default": {
            "fetcher": {"url": {"url": "https://old.invalid/demo"}},
            "hash": "sha256-old",
            "fingerprints": {"hash": "/nix/store/old-source.drv"}
        }}
    });
    fs::write(
        dir.0.join("pins.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "schemaVersion": 2,
            "pins": {"demo": original}
        }))
        .unwrap(),
    )
    .unwrap();
    write_executable(
        &dir.0.join("nix"),
        r#"#!/bin/sh
case "$1" in
  eval)
    case "$*" in
      *p.check*) printf '%s\n' '{"demo":{"cmd":"printf v2"}}' ;;
      *source.fetchSrc.drvPath*)
        case "$*" in
          *sha256-new*) printf '%s\n' '{"demo":{"sources":{"default":{"src":"/nix/store/new-source.drv","fetcher":{"url":{"url":"https://new.invalid/demo"}},"derived":{"api.vendorHash":"/nix/store/api.drv","web.npmDepsHash":"/nix/store/web.drv"},"packages":{"api":{"api.vendorHash":"/nix/store/api.drv"},"web":{"web.npmDepsHash":"/nix/store/web.drv"}}}}}}' ;;
          *) printf '%s\n' '{"demo":{"sources":{"default":{"src":"/nix/store/new-source.drv","fetcher":{"url":{"url":"https://new.invalid/demo"}},"derived":{}}}}}' ;;
        esac
        ;;
    esac
    ;;
  build)
    case "$*" in
      *new-source.drv*) printf '%s\n' 'got: sha256-new' >&2 ;;
      *api.drv*) printf '%s\n' 'api hash failed' >&2 ;;
      *web.drv*) printf '%s\n' 'web hash failed' >&2 ;;
    esac
    exit 1
    ;;
esac
"#,
    );

    let output = run(&dir.0, &["update"]);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(!output.status.success());
    let pins: Value = serde_json::from_slice(&fs::read(dir.0.join("pins.json")).unwrap()).unwrap();
    assert_eq!(pins["pins"]["demo"], original);
    let failure = pins["failures"]["demo"].as_str().unwrap();
    let api = failure.find("Package 'api'").unwrap();
    let web = failure.find("Package 'web'").unwrap();
    assert!(api < web, "{failure}");
    assert_eq!(stderr.matches("⚠ demo").count(), 1, "{stderr}");
}

#[test]
fn npm_deps_failures_explain_how_to_supply_a_missing_lockfile() {
    let _serial = serial();
    let dir = TempDir::new("npm-lockfile");
    fs::write(dir.0.join("pins-config.nix"), "{ pin }: {}\n").unwrap();
    fs::write(
        dir.0.join("pins.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "schemaVersion": 2,
            "pins": {
                "demo": {
                    "version": "v1",
                    "sources": {"default": {
                        "fetcher": {"url": {"url": "https://example.invalid/demo"}},
                        "hash": "sha256-existing",
                        "fingerprints": {"hash": "/nix/store/demo-source.drv"}
                    }}
                }
            }
        }))
        .unwrap(),
    )
    .unwrap();
    write_executable(
        &dir.0.join("nix"),
        r#"#!/bin/sh
case "$1" in
  eval)
    case "$*" in
      *p.check*) printf '%s\n' '{"demo":{"cmd":"printf v1"}}' ;;
      *source.fetchSrc.drvPath*) printf '%s\n' '{"demo":{"sources":{"default":{"src":"/nix/store/demo-source.drv","fetcher":{"url":{"url":"https://example.invalid/demo"}},"derived":{"npmDepsHash":"/nix/store/demo-npm-deps.drv"}}}}}' ;;
    esac
    ;;
  build)
    printf '%s\n' 'ERROR: The package-lock.json file does not exist' >&2
    exit 1
    ;;
esac
"#,
    );

    let output = run(&dir.0, &["update"]);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(!output.status.success());
    assert!(stderr.contains("package-lock.json file does not exist"));
    assert!(stderr.contains("postPatch"));
    assert!(stderr.contains("⚠ demo v1 · npmDepsHash"), "{stderr}");
    assert!(!stderr.contains("default/npmDepsHash"), "{stderr}");
}

#[test]
fn reader_dispatches_all_fetchers() {
    let _serial = serial();
    let dir = TempDir::new("reader");
    let pins_path = dir.0.join("pins.json");
    fs::write(
        &pins_path,
        serde_json::to_vec_pretty(&serde_json::json!({
            "schemaVersion": 2,
            "pins": {
                "github": {
                    "version": "v1",
                    "sources": {
                        "default": {
                            "fetcher": {"github": {"owner": "acme", "repo": "demo", "rev": "v1"}},
                            "hash": "sha256-github",
                            "derived": {"vendorHash": "sha256-vendor"}
                        }
                    }
                },
                "git": {
                    "version": "v2",
                    "sources": {
                        "default": {
                            "fetcher": {"git": {"url": "https://example.invalid/repo.git", "rev": "v2"}},
                            "hash": "sha256-git"
                        }
                    }
                },
                "huggingface": {
                    "version": "v3",
                    "sources": {
                        "default": {
                            "fetcher": {"huggingface": {"repoId": "acme/demo", "rev": "v3", "backend": "lfs"}},
                            "hash": "sha256-huggingface"
                        }
                    }
                },
                "zip": {
                    "version": "v4",
                    "sources": {
                        "default": {
                            "fetcher": {"zip": {"url": "https://example.invalid/demo.zip", "stripRoot": false}},
                            "hash": "sha256-zip"
                        }
                    }
                }
            }
        }))
        .unwrap(),
    )
    .unwrap();

    let reader = Path::new(env!("CARGO_MANIFEST_DIR")).join("pins.nix");
    let expression = format!(
        r#"let
          pins = import {reader} {{
            file = {pins};
            pkgs = {{
    fetchFromGitHub = args: args // {{ kind = "github"; }};
    fetchgit = args: args // {{ kind = "git"; }};
    fetchFromHuggingFace = args: args // {{ kind = "huggingface"; }};
    fetchurl = args: args // {{ kind = "url"; }};
    fetchzip = args: args // {{ kind = "zip"; }};
            }};
          }};
        in {{
          github = {{
            inherit (pins.github.sources.default.src) kind owner repo rev hash;
            inherit (pins.github) version;
            vendorHash = pins.github.sources.default.derived.vendorHash;
          }};
  git = {{
    inherit (pins.git.sources.default.src) kind url rev hash;
    inherit (pins.git) version;
  }};
  huggingface = {{
    inherit (pins.huggingface.sources.default.src) kind repoId rev backend hash;
    inherit (pins.huggingface) version;
  }};
  zip = {{
    inherit (pins.zip.sources.default.src) kind url stripRoot hash;
    inherit (pins.zip) version;
  }};
          pinPromoted = pins.github ? src;
          sourcePromoted = pins.github.sources.default ? vendorHash;
        }}"#,
        reader = reader.display(),
        pins = pins_path.display(),
    );
    let output = Command::new("nix")
        .args(["eval", "--impure", "--json", "--expr", &expression])
        .output()
        .unwrap();

    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["github"]["kind"], "github");
    assert_eq!(result["github"]["version"], "v1");
    assert_eq!(result["github"]["vendorHash"], "sha256-vendor");
    assert_eq!(result["huggingface"]["kind"], "huggingface");
    assert_eq!(result["huggingface"]["backend"], "lfs");
    assert_eq!(result["zip"]["kind"], "zip");
    assert_eq!(result["zip"]["stripRoot"], false);
    assert_eq!(result["git"]["kind"], "git");
    assert_eq!(result["git"]["version"], "v2");
    assert_eq!(result["pinPromoted"], false);
    assert_eq!(result["sourcePromoted"], false);
}
#[test]
fn github_checker_uses_the_overridable_api_base_and_token() {
    let _serial = serial();
    let dir = TempDir::new("github-checker");
    fs::write(dir.0.join("pins-config.nix"), "{ pin }: {}\n").unwrap();
    write_executable(
        &dir.0.join("nix"),
        r#"#!/bin/sh
case "$1" in
  eval)
    case "$*" in
      *p.check*) printf '%s\n' '{"demo":{"github":"acme/demo"}}' ;;
      *source.fetchSrc.drvPath*) printf '%s\n' '{"demo":{"sources":{"default":{"src":"/nix/store/demo-source.drv","fetcher":{"github":{"owner":"acme","repo":"demo","rev":"v9"}},"derived":{}}}}}' ;;
    esac
    ;;
  build)
    printf '%s\n' '  got: sha256-source' >&2
    exit 1
    ;;
esac
"#,
    );
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(std::time::Instant::now() < deadline, "Checker made no HTTP request");
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                Err(error) => panic!("{error}"),
            }
        };
        stream.set_nonblocking(false).unwrap();
        let mut request = [0; 4096];
        let size = stream.read(&mut request).unwrap();
        let request = String::from_utf8_lossy(&request[..size]);
        assert!(request.starts_with("GET /repos/acme/demo/releases/latest "));
        assert!(request.contains("authorization: Bearer secret-token"));
        let body = r#"{"tag_name":"v9"}"#;
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        )
        .unwrap();
    });
    let base = format!("http://{address}");

    let output = run_with_env(
        &dir.0,
        &["update"],
        &[
            ("NIX_PINS_GITHUB_API_BASE", &base),
            ("NIX_PINS_GITHUB_TOKEN", "secret-token"),
        ],
    );
    server.join().unwrap();

    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("secret-token"));
    let pins: Value = serde_json::from_slice(&fs::read(dir.0.join("pins.json")).unwrap()).unwrap();
    assert_eq!(pins["pins"]["demo"]["version"], "v9");
}

#[test]
fn remaining_builtin_checkers_use_their_public_config_shapes() {
    let _serial = serial();
    let dir = TempDir::new("builtin-checkers");
    fs::write(dir.0.join("pins-config.nix"), "{ pin }: {}\n").unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let base = format!("http://{address}");
    let nix_script = format!(
        r#"#!/bin/sh
case "$1" in
  eval)
    case "$*" in
*p.check*) printf '%s\n' '{{"crate":{{"crate":"demo-crate"}},"git":{{"git":{{"url":"https://example.invalid/repo.git","mode":"tag","sort":"semver"}}}},"pypi":{{"pypi":"demo-package"}},"url":{{"url":{{"url":"{base}/versions","regex":"version=([0-9.]+)"}}}}}}' ;;
      *source.fetchSrc.drvPath*) printf '%s\n' '{{"crate":{{"sources":{{"default":{{"src":"/nix/store/crate.drv","fetcher":{{"url":{{"url":"https://example.invalid/crate"}}}},"derived":{{}}}}}}}},"git":{{"sources":{{"default":{{"src":"/nix/store/git.drv","fetcher":{{"git":{{"url":"https://example.invalid/repo.git","rev":"v1.10.0"}}}},"derived":{{}}}}}}}},"pypi":{{"sources":{{"default":{{"src":"/nix/store/pypi.drv","fetcher":{{"url":{{"url":"https://example.invalid/pypi"}}}},"derived":{{}}}}}}}},"url":{{"sources":{{"default":{{"src":"/nix/store/url.drv","fetcher":{{"url":{{"url":"https://example.invalid/url"}}}},"derived":{{}}}}}}}}}}' ;;
    esac
    ;;
  build)
    printf '%s\n' '  got: sha256-source' >&2
    exit 1
    ;;
esac
"#
    );
    write_executable(&dir.0.join("nix"), &nix_script);
    write_executable(
        &dir.0.join("git"),
        r#"#!/bin/sh
printf 'one\trefs/tags/v1.9.0\ntwo\trefs/tags/v1.10.0\n'
"#,
    );
    let server = std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        for _ in 0..3 {
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(std::time::Instant::now() < deadline, "Checker made no HTTP request");
                        std::thread::sleep(std::time::Duration::from_millis(10));
                    }
                    Err(error) => panic!("{error}"),
                }
            };
            stream.set_nonblocking(false).unwrap();
            let mut request = [0; 4096];
            let size = stream.read(&mut request).unwrap();
            let request = String::from_utf8_lossy(&request[..size]);
            let body = if request.starts_with("GET /api/v1/crates/demo-crate ") {
                r#"{"crate":{"newest_version":"2.3.4"}}"#
            } else if request.starts_with("GET /pypi/demo-package/json ") {
                r#"{"info":{"version":"3.4.5"}}"#
            } else if request.starts_with("GET /versions ") {
                "version=4.5.6"
            } else {
                panic!("unexpected request: {request}");
            };
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )
            .unwrap();
        }
    });

    let output = run_with_env(
        &dir.0,
        &["update"],
        &[("NIX_PINS_CRATES_API_BASE", &base), ("NIX_PINS_PYPI_API_BASE", &base)],
    );
    server.join().unwrap();

    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let pins: Value = serde_json::from_slice(&fs::read(dir.0.join("pins.json")).unwrap()).unwrap();
    assert_eq!(pins["pins"]["crate"]["version"], "2.3.4");
    assert_eq!(pins["pins"]["git"]["version"], "v1.10.0");
    assert_eq!(pins["pins"]["pypi"]["version"], "3.4.5");
    assert_eq!(pins["pins"]["url"]["version"], "4.5.6");
}

#[test]
fn github_rate_limits_are_actionable_without_leaking_the_token() {
    let _serial = serial();
    let dir = TempDir::new("github-rate-limit");
    fs::write(dir.0.join("pins-config.nix"), "{ pin }: {}\n").unwrap();
    write_executable(
        &dir.0.join("nix"),
        r#"#!/bin/sh
case "$1" in
  eval) printf '%s\n' '{"demo":{"github":"acme/demo"}}' ;;
esac
"#,
    );
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        for _ in 0..4 {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0; 4096];
            let size = stream.read(&mut request).unwrap();
            assert!(String::from_utf8_lossy(&request[..size]).contains("Bearer secret-token"));
            write!(
                stream,
                "HTTP/1.1 429 Too Many Requests\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            )
            .unwrap();
        }
    });
    let base = format!("http://{address}");

    let output = run_with_env(
        &dir.0,
        &["update"],
        &[
            ("NIX_PINS_GITHUB_API_BASE", &base),
            ("NIX_PINS_GITHUB_TOKEN", "secret-token"),
        ],
    );
    server.join().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(!output.status.success());
    assert!(stderr.contains("rate limit"));
    assert!(!stderr.contains("secret-token"));
}

#[test]
fn checker_stage_runs_concurrently_before_hashing() {
    let _serial = serial();
    let dir = TempDir::new("checker-concurrency");
    fs::write(dir.0.join("pins-config.nix"), "{ pin }: {}\n").unwrap();
    write_executable(
        &dir.0.join("nix"),
        r#"#!/bin/sh
case "$1" in
  eval)
    case "$*" in
      *p.check*) printf '%s\n' '{"a":{"cmd":"sleep 0.6; printf v1"},"b":{"cmd":"sleep 0.6; printf v1"},"c":{"cmd":"sleep 0.6; printf v1"},"d":{"cmd":"sleep 0.6; printf v1"}}' ;;
      *source.fetchSrc.drvPath*) printf '%s\n' '{"a":{"sources":{"default":{"src":"/nix/store/a.drv","fetcher":{"url":{"url":"https://example.invalid/a"}},"derived":{}}}},"b":{"sources":{"default":{"src":"/nix/store/b.drv","fetcher":{"url":{"url":"https://example.invalid/b"}},"derived":{}}}},"c":{"sources":{"default":{"src":"/nix/store/c.drv","fetcher":{"url":{"url":"https://example.invalid/c"}},"derived":{}}}},"d":{"sources":{"default":{"src":"/nix/store/d.drv","fetcher":{"url":{"url":"https://example.invalid/d"}},"derived":{}}}}}' ;;
    esac
    ;;
  build)
    printf '%s\n' '  got: sha256-source' >&2
    exit 1
    ;;
esac
"#,
    );

    let started = std::time::Instant::now();
    let output = run(&dir.0, &["update"]);
    let elapsed = started.elapsed();

    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(
        elapsed < std::time::Duration::from_millis(3200),
        "Checker stage took {elapsed:?}"
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("Processed 4 pins; 0 failed"));
}

#[test]
fn download_stage_is_concurrent_by_default() {
    let _serial = serial();
    let dir = TempDir::new("hash-concurrency");
    fs::write(dir.0.join("pins-config.nix"), "{ pin }: {}\n").unwrap();
    let nix_script = r#"#!/bin/sh
case "$1" in
  eval)
    case "$*" in
      *p.check*) printf '%s\n' '{"a":{"cmd":"printf v1"},"b":{"cmd":"printf v1"}}' ;;
      *source.fetchSrc.drvPath*) printf '%s\n' '{"a":{"sources":{"default":{"src":"/nix/store/a.drv","fetcher":{"url":{"url":"https://example.invalid/a"}},"derived":{}}}},"b":{"sources":{"default":{"src":"/nix/store/b.drv","fetcher":{"url":{"url":"https://example.invalid/b"}},"derived":{}}}}}' ;;
    esac
    ;;
  build)
    touch "__DIR__/build-$$"
    for attempt in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20; do
      set -- "__DIR__"/build-*
      [ "$#" -ge 2 ] && break
      sleep 0.05
    done
    [ "$#" -ge 2 ] || {
      printf '%s\n' 'hash builds were not concurrent' >&2
      exit 99
    }
    printf '%s\n' '  got: sha256-source' >&2
    exit 1
    ;;
esac
"#
    .replace("__DIR__", &dir.0.display().to_string());
    write_executable(&dir.0.join("nix"), &nix_script);

    let output = run(&dir.0, &["update"]);

    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
}

#[test]
fn update_accepts_exact_names_and_a_regex_filter() {
    let _serial = serial();
    let dir = TempDir::new("filter");
    fs::write(dir.0.join("pins-config.nix"), "{ pin }: {}\n").unwrap();
    write_executable(
        &dir.0.join("nix"),
        r#"#!/bin/sh
case "$1" in
  eval)
    case "$*" in
      *p.check*) printf '%s\n' '{"alpha":{"cmd":"printf v1"},"beta":{"cmd":"printf v1"}}' ;;
      *source.fetchSrc.drvPath*) printf '%s\n' '{"alpha":{"sources":{"default":{"src":"/nix/store/alpha.drv","fetcher":{"url":{"url":"https://example.invalid/alpha"}},"derived":{}}}}}' ;;
    esac
    ;;
  build)
    printf '%s\n' '  got: sha256-alpha' >&2
    exit 1
    ;;
esac
"#,
    );

    let output = run(&dir.0, &["update", "--filter", "^alpha$"]);

    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let pins: Value = serde_json::from_slice(&fs::read(dir.0.join("pins.json")).unwrap()).unwrap();
    assert!(pins["pins"].get("alpha").is_some());
    assert!(pins["pins"].get("beta").is_none());
}

#[test]
fn clap_help_version_and_errors_use_standard_exit_codes() {
    let dir = TempDir::new("clap");

    let help = run(&dir.0, &["--help"]);
    assert!(help.status.success(), "{}", String::from_utf8_lossy(&help.stderr));
    let help = String::from_utf8_lossy(&help.stdout);
    assert!(help.contains("Usage:"), "{help}");
    assert!(help.contains("update"), "{help}");
    assert!(help.contains("status"), "{help}");

    let version = run(&dir.0, &["--version"]);
    assert!(version.status.success());
    assert!(String::from_utf8_lossy(&version.stdout).contains("nix-pins 0.1.0"));

    let error = run(&dir.0, &["update", "--unknown"]);
    assert_eq!(error.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&error.stderr).contains("--unknown"));
}

#[test]
fn global_config_and_pins_paths_are_relocatable() {
    let _serial = serial();
    let dir = TempDir::new("paths");
    fs::create_dir_all(dir.0.join("config")).unwrap();
    fs::create_dir_all(dir.0.join("state")).unwrap();
    fs::write(dir.0.join("config/custom.nix"), "{ pin }: {}\n").unwrap();

    let output = run(
        &dir.0,
        &["--config", "config/custom.nix", "status", "--pins", "state/custom.json"],
    );

    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
}

#[test]
fn config_and_pins_paths_can_come_from_the_environment() {
    let _serial = serial();
    let dir = TempDir::new("path-env");
    fs::create_dir_all(dir.0.join("config")).unwrap();
    fs::create_dir_all(dir.0.join("state")).unwrap();
    fs::write(dir.0.join("config/custom.nix"), "{ pin }: {}\n").unwrap();

    let output = run_with_env(
        &dir.0,
        &["status"],
        &[
            ("NIX_PINS_CONFIG", "config/custom.nix"),
            ("NIX_PINS_FILE", "state/custom.json"),
        ],
    );
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));

    let output = run_with_env(
        &dir.0,
        &["--config", "config/custom.nix", "--pins", "state/custom.json", "status"],
        &[("NIX_PINS_CONFIG", "missing.nix"), ("NIX_PINS_FILE", "missing.json")],
    );
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
}

#[test]
fn selectors_reject_unknown_names_and_filter_only_zero_matches() {
    let _serial = serial();
    let dir = TempDir::new("strict-selection");
    fs::write(dir.0.join("pins-config.nix"), "{ pin }: {}\n").unwrap();
    write_executable(
        &dir.0.join("nix"),
        r#"#!/bin/sh
printf '%s\n' '{"demo":{"cmd":"printf v1"}}'
"#,
    );

    let unknown = run(&dir.0, &["status", "missing"]);
    assert_eq!(unknown.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&unknown.stderr).contains("unknown pin name"));

    let empty = run(&dir.0, &["status", "--filter", "^missing$"]);
    assert_eq!(empty.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&empty.stderr).contains("matched no pins"));
}

#[test]
fn full_update_prunes_removed_pins_but_selected_update_preserves_them() {
    let _serial = serial();
    let dir = TempDir::new("reconcile");
    fs::write(dir.0.join("pins-config.nix"), "{ pin }: {}\n").unwrap();
    fs::write(
        dir.0.join("pins.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "schemaVersion": 2,
            "pins": {
                "alpha": {
                    "version": "old",
                    "sources": {"default": {
                        "fetcher": {"url": {"url": "https://old.invalid/alpha"}},
                        "hash": "sha256-old-alpha",
                        "fingerprints": {"hash": "/nix/store/old-alpha.drv"}
                    }}
                },
                "beta": {
                    "version": "old",
                    "sources": {"default": {
                        "fetcher": {"url": {"url": "https://old.invalid/beta"}},
                        "hash": "sha256-old-beta",
                        "fingerprints": {"hash": "/nix/store/old-beta.drv"}
                    }}
                },
                "removed": {
                    "version": "old",
                    "sources": {"default": {
                        "fetcher": {"url": {"url": "https://old.invalid/removed"}},
                        "hash": "sha256-old-removed"
                    }}
                }
            },
            "failures": {"removed": "old failure"}
        }))
        .unwrap(),
    )
    .unwrap();
    write_executable(
        &dir.0.join("nix"),
        r#"#!/bin/sh
case "$1" in
  eval)
    case "$*" in
      *p.check*) printf '%s\n' '{"alpha":{"cmd":"printf v2"},"beta":{"cmd":"printf v2"}}' ;;
      *source.fetchSrc.drvPath*) printf '%s\n' '{"alpha":{"sources":{"default":{"src":"/nix/store/alpha.drv","fetcher":{"url":{"url":"https://new.invalid/alpha"}},"derived":{}}}},"beta":{"sources":{"default":{"src":"/nix/store/beta.drv","fetcher":{"url":{"url":"https://new.invalid/beta"}},"derived":{}}}}}' ;;
    esac
    ;;
  build)
    printf '%s\n' 'got: sha256-new' >&2
    exit 1
    ;;
esac
"#,
    );

    let selected = run(&dir.0, &["update", "alpha"]);
    assert!(
        selected.status.success(),
        "{}",
        String::from_utf8_lossy(&selected.stderr)
    );
    let pins: Value = serde_json::from_slice(&fs::read(dir.0.join("pins.json")).unwrap()).unwrap();
    assert!(pins["pins"].get("removed").is_some());
    assert!(pins["failures"].get("removed").is_some());

    let full = run(&dir.0, &["update"]);
    assert!(full.status.success(), "{}", String::from_utf8_lossy(&full.stderr));
    let pins: Value = serde_json::from_slice(&fs::read(dir.0.join("pins.json")).unwrap()).unwrap();
    assert!(pins["pins"].get("removed").is_none());
    assert!(pins.get("failures").is_none());
}

#[test]
fn first_pin_failure_records_only_the_failure() {
    let _serial = serial();
    let dir = TempDir::new("first-failure");
    fs::write(dir.0.join("pins-config.nix"), "{ pin }: {}\n").unwrap();
    write_executable(
        &dir.0.join("nix"),
        r#"#!/bin/sh
printf '%s\n' '{"demo":{"cmd":"printf failed >&2; exit 1"}}'
"#,
    );

    let output = run(&dir.0, &["update"]);
    assert_eq!(output.status.code(), Some(1));
    let pins: Value = serde_json::from_slice(&fs::read(dir.0.join("pins.json")).unwrap()).unwrap();
    assert!(pins["pins"].get("demo").is_none());
    assert!(pins["failures"]["demo"].as_str().unwrap().contains("failed"));
}

#[test]
fn configuration_errors_leave_the_pins_file_byte_for_byte_unchanged() {
    let _serial = serial();
    let dir = TempDir::new("config-error");
    fs::write(dir.0.join("pins-config.nix"), "{ pin }: {}\n").unwrap();
    let original = b"{\n  \"schemaVersion\": 2,\n  \"pins\": {}\n}\n";
    fs::write(dir.0.join("pins.json"), original).unwrap();
    write_executable(
        &dir.0.join("nix"),
        r#"#!/bin/sh
printf '%s\n' 'invalid fetcher mapping' >&2
exit 1
"#,
    );

    let output = run(&dir.0, &["update"]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1));
    assert!(stderr.contains("Loading configuration failed"), "{stderr}");
    assert!(stderr.contains("nix-pins:"), "{stderr}");
    assert!(!stderr.contains("Processed 0 pins"), "{stderr}");
    assert!(!stderr.contains("demo failed"), "{stderr}");
    assert_eq!(fs::read(dir.0.join("pins.json")).unwrap(), original);
}

#[test]
fn unchanged_update_preserves_the_pins_file_mtime() {
    let _serial = serial();
    let dir = TempDir::new("mtime");
    fs::write(dir.0.join("pins-config.nix"), "{ pin }: {}\n").unwrap();
    write_executable(
        &dir.0.join("nix"),
        r#"#!/bin/sh
case "$1" in
  eval)
    case "$*" in
      *p.check*) printf '%s\n' '{"demo":{"cmd":"printf v1"}}' ;;
      *source.fetchSrc.drvPath*) printf '%s\n' '{"demo":{"sources":{"default":{"src":"/nix/store/demo.drv","fetcher":{"url":{"url":"https://example.invalid/demo"}},"derived":{}}}}}' ;;
    esac
    ;;
  build)
    printf '%s\n' 'got: sha256-demo' >&2
    exit 1
    ;;
esac
"#,
    );

    let first = run(&dir.0, &["update"]);
    assert!(first.status.success());
    let modified = fs::metadata(dir.0.join("pins.json")).unwrap().modified().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(20));
    let second = run(&dir.0, &["update"]);
    assert!(second.status.success());
    assert_eq!(
        fs::metadata(dir.0.join("pins.json")).unwrap().modified().unwrap(),
        modified
    );
}

#[test]
fn update_consumes_internal_json_without_leaking_activity_urls() {
    let _serial = serial();
    let dir = TempDir::new("internal-json");
    fs::write(dir.0.join("pins-config.nix"), "{ pin }: {}\n").unwrap();
    write_executable(
        &dir.0.join("nix"),
        r#"#!/bin/sh
case "$1" in
  eval)
    case "$*" in
      *p.check*) printf '%s\n' '{"demo":{"cmd":"printf v1"}}' ;;
      *source.fetchSrc.drvPath*) printf '%s\n' '{"demo":{"sources":{"default":{"src":"/nix/store/demo.drv","fetcher":{"url":{"url":"https://example.invalid/demo"}},"derived":{},"packages":{}}}}}' ;;
    esac
    ;;
  build)
    case "$*" in
      *"--log-format internal-json"*) ;;
      *) printf '%s\n' 'missing internal-json log format' >&2; exit 99 ;;
    esac
printf '%s\n' '@nix {"action":"start","id":1,"type":101,"fields":["https://secret.invalid/archive"]}' >&2
printf '%s\n' '@nix {"action":"result","id":1,"type":105,"fields":[1048576,2097152,0,0]}' >&2
printf '%s\n' '@nix {"action":"start","id":2,"type":0,"fields":[]}' >&2
printf '%s\n' '@nix {"action":"start","id":3,"type":102,"fields":[]}' >&2
printf '%s\n' '@nix {"action":"start","id":4,"type":103,"fields":[]}' >&2
printf '%s\n' '@nix {"action":"start","id":5,"type":104,"fields":[]}' >&2
printf '%s\n' '@nix {"action":"result","id":2,"type":105,"fields":[0,1,0,0]}' >&2
printf '%s\n' '@nix {"action":"result","id":2,"type":106,"fields":[101,0]}' >&2
printf '            got:    \033[35;1msha256-NWcqJkIPRKGSr9n6X2DWlS4/Kzsg+k4ue+mMuo/drn4=\033[0m\n' >&2
exit 1
    ;;
esac
"#,
    );

    let output = run(&dir.0, &["update"]);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(output.status.success(), "{stderr}");
    assert!(stderr.contains("demo — → v1"), "{stderr}");
    assert!(stderr.contains("Writing pins.json done"), "{stderr}");
    assert!(!stderr.contains("secret.invalid"), "{stderr}");
    assert!(!stderr.contains("@nix"), "{stderr}");
    assert!(!stderr.contains("\u{1b}"), "{stderr}");
}
