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
      *p.fetchSrc.drvPath*)
        printf '%s\n' '{"demo":{"src":"/nix/store/demo-source.drv","fetcher":{"github":{"owner":"acme","repo":"demo","rev":"v1.2.3"}},"derived":{}}}'
        ;;
      *)
        printf '%s\n' 'unexpected eval' >&2
        exit 2
        ;;
    esac
    ;;
  build)
    printf '%s\n' 'error: hash mismatch' '  got: sha256-BlpIDik4hkU4c+KCyAmgUURIN362RDQID/qo6Ojp2Ek=' >&2
    exit 1
    ;;
esac
"#,
    );

    let output = run(&dir.0, &["update"]);

    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let pins: Value = serde_json::from_slice(&fs::read(dir.0.join("pins.json")).unwrap()).unwrap();
    assert_eq!(pins["schemaVersion"], 1);
    assert_eq!(pins["pins"]["demo"]["version"], "v1.2.3");
    assert_eq!(
        pins["pins"]["demo"]["hash"],
        "sha256-BlpIDik4hkU4c+KCyAmgUURIN362RDQID/qo6Ojp2Ek="
    );
    assert_eq!(
        pins["pins"]["demo"]["fetcher"]["github"],
        serde_json::json!({"owner": "acme", "repo": "demo", "rev": "v1.2.3"})
    );
    assert_eq!(
        pins["pins"]["demo"]["fingerprints"]["hash"],
        "/nix/store/demo-source.drv"
    );
    assert!(pins["pins"]["demo"].get("derived").is_none());
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
        pins["pins"]["demo"]["hash"],
        "sha256-BlpIDik4hkU4c+KCyAmgUURIN362RDQID/qo6Ojp2Ek="
    );
    assert_eq!(
        pins["pins"]["demo"]["fetcher"],
        serde_json::json!({
            "url": {
                "url": "https://example.com/demo-v1.2.3.tar.gz"
            }
        })
    );
    assert!(pins["pins"]["demo"]["fingerprints"]["hash"]
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
            "schemaVersion": 1,
            "pins": {
                "bad": {
                    "version": "v1",
                    "fetcher": {"url": {"url": "https://old.invalid/bad"}},
                    "hash": "sha256-old-bad",
                    "fingerprints": {"hash": "/nix/store/old-bad.drv"}
                },
                "good": {
                    "version": "v1",
                    "fetcher": {"url": {"url": "https://old.invalid/good"}},
                    "hash": "sha256-old-good",
                    "fingerprints": {"hash": "/nix/store/old-good.drv"}
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
      *p.fetchSrc.drvPath*)
        printf '%s\n' '{"good":{"src":"/nix/store/new-good.drv","fetcher":{"url":{"url":"https://new.invalid/good"}},"derived":{}}}'
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
    assert!(stderr.contains("Processed 2 pins; 1 failed"), "{stderr}");
    assert!(stderr.contains("nix-pins: update completed with failures:"), "{stderr}");
    assert!(stderr.contains("bad"), "{stderr}");
    assert!(stderr.contains("checker-failed"), "{stderr}");
    let pins: Value = serde_json::from_slice(&fs::read(dir.0.join("pins.json")).unwrap()).unwrap();
    assert_eq!(pins["pins"]["bad"]["version"], "v1");
    assert_eq!(pins["pins"]["bad"]["hash"], "sha256-old-bad");
    assert_eq!(pins["pins"]["good"]["version"], "v2");
    assert_eq!(pins["pins"]["good"]["hash"], "sha256-new-good");
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
      *p.fetchSrc.drvPath*)
        printf '%s\n' '{"demo":{"src":"/nix/store/demo-source.drv","fetcher":{"url":{"url":"https://example.invalid/demo"}},"derived":{}}}'
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
            "schemaVersion": 1,
            "pins": {
                "bad": {
                    "version": "v1",
                    "fetcher": {"url": {"url": "https://example.invalid/bad"}},
                    "hash": "sha256-bad"
                },
                "good": {
                    "version": "v2",
                    "fetcher": {"url": {"url": "https://example.invalid/good"}},
                    "hash": "sha256-good"
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
            "schemaVersion": 1,
            "pins": {
                "demo": {
                    "version": "v1",
                    "fetcher": {"url": {"url": "https://example.invalid/demo"}},
                    "hash": "sha256-existing",
                    "fingerprints": {"hash": "/nix/store/unchanged-source.drv"}
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
      *p.fetchSrc.drvPath*)
        printf '%s\n' '{"demo":{"src":"/nix/store/unchanged-source.drv","fetcher":{"url":{"url":"https://example.invalid/demo"}},"derived":{}}}'
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
    assert_eq!(pins["pins"]["demo"]["hash"], "sha256-existing");
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
      *p.fetchSrc.drvPath*)
        case "$*" in
          *sha256-source*)
            printf '%s\n' '{"demo":{"src":"/nix/store/demo-source.drv","fetcher":{"github":{"owner":"acme","repo":"demo","rev":"v1"}},"derived":{"vendorHash":"/nix/store/demo-go-modules.drv","npmDepsHash":"/nix/store/demo-npm-deps.drv"}}}'
            ;;
          *)
            printf '%s\n' '{"demo":{"src":"/nix/store/demo-source.drv","fetcher":{"github":{"owner":"acme","repo":"demo","rev":"v1"}},"derived":{"vendorHash":"/nix/store/source-mismatch.drv","npmDepsHash":"/nix/store/source-mismatch.drv"}}}'
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
    assert_eq!(pins["pins"]["demo"]["hash"], "sha256-source");
    assert_eq!(pins["pins"]["demo"]["derived"]["vendorHash"], "sha256-vendor");
    assert_eq!(pins["pins"]["demo"]["derived"]["npmDepsHash"], "sha256-npm");
    assert_eq!(
        pins["pins"]["demo"]["fingerprints"]["vendorHash"],
        "/nix/store/demo-go-modules.drv"
    );
    assert_eq!(
        pins["pins"]["demo"]["fingerprints"]["npmDepsHash"],
        "/nix/store/demo-npm-deps.drv"
    );

    write_executable(
        &nix,
        r#"#!/bin/sh
case "$1" in
  eval)
    case "$*" in
      *p.check*) printf '%s\n' '{"demo":{"cmd":"printf v1"}}' ;;
      *p.fetchSrc.drvPath*) printf '%s\n' '{"demo":{"src":"/nix/store/demo-source.drv","fetcher":{"github":{"owner":"acme","repo":"demo","rev":"v1"}},"derived":{"vendorHash":"/nix/store/demo-go-modules.drv","npmDepsHash":"/nix/store/demo-npm-deps.drv"}}}' ;;
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
    assert!(stderr.contains("Reused vendorHash"), "{stderr}");
    assert!(stderr.contains("Reused npmDepsHash"), "{stderr}");
}

#[test]
fn npm_deps_failures_explain_how_to_supply_a_missing_lockfile() {
    let _serial = serial();
    let dir = TempDir::new("npm-lockfile");
    fs::write(dir.0.join("pins-config.nix"), "{ pin }: {}\n").unwrap();
    fs::write(
        dir.0.join("pins.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "schemaVersion": 1,
            "pins": {
                "demo": {
                    "version": "v1",
                    "fetcher": {"url": {"url": "https://example.invalid/demo"}},
                    "hash": "sha256-existing",
                    "fingerprints": {"hash": "/nix/store/demo-source.drv"}
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
      *p.fetchSrc.drvPath*) printf '%s\n' '{"demo":{"src":"/nix/store/demo-source.drv","fetcher":{"url":{"url":"https://example.invalid/demo"}},"derived":{"npmDepsHash":"/nix/store/demo-npm-deps.drv"}}}' ;;
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
    assert!(stderr.contains("Failed · npmDepsHash"), "{stderr}");
    assert!(!stderr.contains("default/npmDepsHash"), "{stderr}");
}

#[test]
fn reader_dispatches_github_and_git_fetchers() {
    let _serial = serial();
    let dir = TempDir::new("reader");
    let pins_path = dir.0.join("pins.json");
    fs::write(
        &pins_path,
        serde_json::to_vec_pretty(&serde_json::json!({
            "schemaVersion": 1,
            "pins": {
                "github": {
                    "version": "v1",
                    "fetcher": {"github": {"owner": "acme", "repo": "demo", "rev": "v1"}},
                    "hash": "sha256-github",
                    "derived": {"vendorHash": "sha256-vendor"}
                },
                "git": {
                    "version": "v2",
                    "fetcher": {"git": {"url": "https://example.invalid/repo.git", "rev": "v2"}},
                    "hash": "sha256-git"
                }
            }
        }))
        .unwrap(),
    )
    .unwrap();
    let reader = Path::new(env!("CARGO_MANIFEST_DIR")).join("pins.nix");
    let expression = format!(
        r#"let pins = import {reader} {{
          file = {pins};
          pkgs = {{
            fetchFromGitHub = args: args // {{ kind = "github"; }};
            fetchgit = args: args // {{ kind = "git"; }};
            fetchurl = args: args // {{ kind = "url"; }};
          }};
        }}; in {{
        github = {{ inherit (pins.github.src) kind owner repo rev hash; inherit (pins.github) pname vendorHash; }};
        git = {{ inherit (pins.git.src) kind url rev hash; inherit (pins.git) pname; }};
        }}"#,
        reader = reader.display(),
        pins = pins_path.display(),
    );
    let output = Command::new("nix")
        .args(["eval", "--impure", "--json", "--expr", &expression])
        .output()
        .unwrap();

    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["github"]["kind"], "github");
    assert_eq!(value["github"]["pname"], "github");
    assert_eq!(value["github"]["vendorHash"], "sha256-vendor");
    assert_eq!(value["git"]["kind"], "git");
    assert_eq!(value["git"]["pname"], "git");
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
      *p.fetchSrc.drvPath*) printf '%s\n' '{"demo":{"src":"/nix/store/demo-source.drv","fetcher":{"github":{"owner":"acme","repo":"demo","rev":"v9"}},"derived":{}}}' ;;
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
      *p.fetchSrc.drvPath*) printf '%s\n' '{{"crate":{{"src":"/nix/store/crate.drv","fetcher":{{"url":{{"url":"https://example.invalid/crate"}}}},"derived":{{}}}},"git":{{"src":"/nix/store/git.drv","fetcher":{{"git":{{"url":"https://example.invalid/repo.git","rev":"v1.10.0"}}}},"derived":{{}}}},"pypi":{{"src":"/nix/store/pypi.drv","fetcher":{{"url":{{"url":"https://example.invalid/pypi"}}}},"derived":{{}}}},"url":{{"src":"/nix/store/url.drv","fetcher":{{"url":{{"url":"https://example.invalid/url"}}}},"derived":{{}}}}}}' ;;
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
      *p.fetchSrc.drvPath*) printf '%s\n' '{"a":{"src":"/nix/store/a.drv","fetcher":{"url":{"url":"https://example.invalid/a"}},"derived":{}},"b":{"src":"/nix/store/b.drv","fetcher":{"url":{"url":"https://example.invalid/b"}},"derived":{}},"c":{"src":"/nix/store/c.drv","fetcher":{"url":{"url":"https://example.invalid/c"}},"derived":{}},"d":{"src":"/nix/store/d.drv","fetcher":{"url":{"url":"https://example.invalid/d"}},"derived":{}}}' ;;
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
    assert!(String::from_utf8_lossy(&output.stderr).contains("4 hashes need recalculation"));
}

#[test]
fn hash_stage_has_an_independent_concurrency_switch() {
    let _serial = serial();
    let dir = TempDir::new("hash-concurrency");
    fs::write(dir.0.join("pins-config.nix"), "{ pin }: {}\n").unwrap();
    let nix_script = r#"#!/bin/sh
case "$1" in
  eval)
    case "$*" in
      *p.check*) printf '%s\n' '{"a":{"cmd":"printf v1"},"b":{"cmd":"printf v1"}}' ;;
      *p.fetchSrc.drvPath*) printf '%s\n' '{"a":{"src":"/nix/store/a.drv","fetcher":{"url":{"url":"https://example.invalid/a"}},"derived":{}},"b":{"src":"/nix/store/b.drv","fetcher":{"url":{"url":"https://example.invalid/b"}},"derived":{}}}' ;;
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

    let output = run_with_env(&dir.0, &["update"], &[("NIX_PINS_HASH_JOBS", "2")]);

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
      *p.fetchSrc.drvPath*) printf '%s\n' '{"alpha":{"src":"/nix/store/alpha.drv","fetcher":{"url":{"url":"https://example.invalid/alpha"}},"derived":{}}}' ;;
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
            "schemaVersion": 1,
            "pins": {
                "alpha": {
                    "version": "old",
                    "fetcher": {"url": {"url": "https://old.invalid/alpha"}},
                    "hash": "sha256-old-alpha",
                    "fingerprints": {"hash": "/nix/store/old-alpha.drv"}
                },
                "beta": {
                    "version": "old",
                    "fetcher": {"url": {"url": "https://old.invalid/beta"}},
                    "hash": "sha256-old-beta",
                    "fingerprints": {"hash": "/nix/store/old-beta.drv"}
                },
                "removed": {
                    "version": "old",
                    "fetcher": {"url": {"url": "https://old.invalid/removed"}},
                    "hash": "sha256-old-removed"
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
      *p.fetchSrc.drvPath*) printf '%s\n' '{"alpha":{"src":"/nix/store/alpha.drv","fetcher":{"url":{"url":"https://new.invalid/alpha"}},"derived":{}},"beta":{"src":"/nix/store/beta.drv","fetcher":{"url":{"url":"https://new.invalid/beta"}},"derived":{}}}' ;;
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
    let original = b"{\n  \"schemaVersion\": 1,\n  \"pins\": {}\n}\n";
    fs::write(dir.0.join("pins.json"), original).unwrap();
    write_executable(
        &dir.0.join("nix"),
        r#"#!/bin/sh
case "$*" in
  *p.check*) printf '%s\n' '{"demo":{"cmd":"printf v1"}}' ;;
  *) printf '%s\n' 'invalid fetcher mapping' >&2; exit 1 ;;
esac
"#,
    );

    let output = run(&dir.0, &["update"]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1));
    assert!(stderr.contains("✗ Resolving sources · 1 pins"), "{stderr}");
    assert!(!stderr.contains("✗ demo"), "{stderr}");
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
      *p.fetchSrc.drvPath*) printf '%s\n' '{"demo":{"src":"/nix/store/demo.drv","fetcher":{"url":{"url":"https://example.invalid/demo"}},"derived":{}}}' ;;
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
      *p.fetchSrc.drvPath*) printf '%s\n' '{"demo":{"src":"/nix/store/demo.drv","fetcher":{"url":{"url":"https://example.invalid/demo"}},"derived":{},"packages":{}}}' ;;
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
    assert!(stderr.contains("demo — v1 Hashing source"), "{stderr}");
    assert!(stderr.contains("✓ demo — v1 Done"), "{stderr}");
    assert!(stderr.contains("✓ Writing pins.json"), "{stderr}");
    assert!(!stderr.contains("secret.invalid"), "{stderr}");
    assert!(!stderr.contains("@nix"), "{stderr}");
    assert!(!stderr.contains("\u{1b}"), "{stderr}");
}
