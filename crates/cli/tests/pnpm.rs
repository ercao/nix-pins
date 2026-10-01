use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

struct Project(PathBuf);

impl Project {
    fn new(config: &str) -> Self {
        static NEXT_ID: AtomicUsize = AtomicUsize::new(0);
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("nix-pins-pnpm-{}-{nonce}-{id}", std::process::id()));
        fs::create_dir(&path).unwrap();
        fs::write(path.join("pins-config.nix"), config).unwrap();
        fs::write(
            path.join("pins.json"),
            serde_json::to_vec(&json!({
                "schemaVersion": 2,
                "pins": {"demo": {
                    "version": "1.0.0",
                    "sources": {"default": {
                        "fetcher": {"github": {"owner": "acme", "repo": "demo", "rev": "1.0.0"}},
                        "hash": "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
                        "derived": {"pnpmDepsHash": "sha256-BlpIDik4hkU4c+KCyAmgUURIN362RDQID/qo6Ojp2Ek="}
                    }}
                }}
            }))
            .unwrap(),
        )
        .unwrap();
        Self(path)
    }

    fn read(&self, expression: &str) -> Value {
        let reader = concat!(env!("CARGO_MANIFEST_DIR"), "/../../nix/pins.nix");
        let expression = format!(
            "let pkgs = import <nixpkgs> {{}}; pins = import {reader} {{ inherit pkgs; config = {}/pins-config.nix; file = {}/pins.json; }}; source = pins.demo.sources.default; in {expression}",
            self.0.display(),
            self.0.display()
        );
        let output = Command::new("nix")
            .args(["eval", "--impure", "--json", "--expr", &expression])
            .output()
            .unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        serde_json::from_slice(&output.stdout).unwrap()
    }

    fn update(&self) -> Output {
        Command::new(env!("CARGO_BIN_EXE_nix-pins"))
            .args(["update", "--config", "pins-config.nix", "--pins", "pins.json"])
            .current_dir(&self.0)
            .output()
            .unwrap()
    }

    fn use_source(&self, source: &Path, root: &str, post_patch: &str) {
        let archive = self.0.join("source.tar.gz");
        let output = Command::new("tar")
            .arg("-czf")
            .arg(&archive)
            .arg("-C")
            .arg(source)
            .arg(".")
            .output()
            .unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let output = Command::new("nix")
            .args(["store", "add-file"])
            .arg(&archive)
            .output()
            .unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let archive = String::from_utf8(output.stdout).unwrap();
        fs::write(
            self.0.join("pins-config.nix"),
            format!(
                r#"{{ pin }}: {{
  demo = pin.mk {{
    checker = pin.checker.cmd "printf 1.0.0";
    fetcher = pin.fetcher.zip {{
      target = _: "file://${{builtins.storePath {archive}}}";
      fetcherArgs.stripRoot = false;
    }};
    postPatch = {post_patch};
    packages.default = pin.pnpmPackage {{ root = {root}; fetcherVersion = 4; }};
  }};
}}"#,
                archive = archive.trim(),
                root = serde_json::to_string(root).unwrap(),
                post_patch = serde_json::to_string(post_patch).unwrap(),
            ),
        )
        .unwrap();
    }

    fn build(&self, root: &str) -> PathBuf {
        let reader = concat!(env!("CARGO_MANIFEST_DIR"), "/../../nix/pins.nix");
        let expression = format!(
            r#"let
  pkgs = import <nixpkgs> {{}};
  pins = import {reader} {{ inherit pkgs; config = {project}/pins-config.nix; file = {project}/pins.json; }};
  source = pins.demo.sources.default;
in pkgs.stdenvNoCC.mkDerivation {{
  pname = "nix-pins-pnpm-consumer";
  version = "1.0.0";
  inherit (source) src;
  pnpmDeps = source.packages.default.pnpmDeps;
  pnpmRoot = {root};
  nativeBuildInputs = [ pkgs.nodejs pkgs.pnpm pkgs.pnpmConfigHook ];
  buildPhase = "cd " + pkgs.lib.escapeShellArg {root} + "; pnpm build";
  installPhase = "mkdir -p $out; cp result.txt $out/result.txt";
}}"#,
            project = self.0.display(),
            root = serde_json::to_string(root).unwrap(),
        );
        let output = Command::new("nix")
            .args(["build", "--impure", "--json", "--no-link", "--expr", &expression])
            .output()
            .unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let result: Value = serde_json::from_slice(&output.stdout).unwrap();
        PathBuf::from(result[0]["outputs"]["out"].as_str().unwrap())
    }
}

impl Drop for Project {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn reader_exposes_locked_pnpm_dependencies_with_the_default_tool() {
    let project = Project::new(
        r#"{ pin }: {
  demo = pin.github {
    target = "acme/demo";
    packages.default = pin.pnpmPackage { root = "."; fetcherVersion = 4; };
  };
}"#,
    );
    let result = project.read(
        r#"let deps = source.packages.default.pnpmDeps; in {
  hash = deps.outputHash;
  format = deps.fetcherVersion;
  usesDefaultPnpm = builtins.any (input: input.drvPath == pkgs.pnpm.drvPath) deps.nativeBuildInputs;
  drv = deps.drvPath;
}"#,
    );
    assert_eq!(result["hash"], "sha256-BlpIDik4hkU4c+KCyAmgUURIN362RDQID/qo6Ojp2Ek=");
    assert_eq!(result["format"], 4);
    assert_eq!(result["usesDefaultPnpm"], true);
    assert!(result["drv"].as_str().unwrap().ends_with(".drv"));
}

#[test]
fn invalid_pnpm_fields_are_configuration_errors_before_any_update() {
    for (arguments, field) in [
        ("fetcherVersion = 4;", "root"),
        ("root = \".\";", "fetcherVersion"),
        ("root = null; fetcherVersion = 4;", "root"),
        ("root = \".\"; fetcherVersion = null;", "fetcherVersion"),
        (
            "root = \".\"; fetcherVersion = 4; pnpmWorkspaces = [\"web\"];",
            "pnpmWorkspaces",
        ),
        (
            "root = \".\"; fetcherVersion = 4; pnpmInstallFlags = [\"--filter=web\"];",
            "pnpmInstallFlags",
        ),
        (
            "root = \".\"; fetcherVersion = 4; pnpmInstallFlags = [\"--filter-prod=web\"];",
            "pnpmInstallFlags",
        ),
    ] {
        let project = Project::new(&format!(
            "{{ pin }}: {{ demo = pin.github {{ target = \"acme/demo\"; packages.web = pin.pnpmPackage {{ {arguments} }}; }}; }}"
        ));
        let before = fs::read(project.0.join("pins.json")).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_nix-pins"))
            .args(["status", "--config", "pins-config.nix", "--pins", "pins.json"])
            .current_dir(&project.0)
            .output()
            .unwrap();
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success(), "无效 {field} 时必须拒绝配置");
        assert!(
            error.contains("demo") && error.contains("web") && error.contains(field),
            "{error}"
        );
        assert_eq!(fs::read(project.0.join("pins.json")).unwrap(), before);
    }
}

#[test]
fn named_pnpm_packages_keep_their_hashes_and_tool_overrides() {
    let project = Project::new(
        r#"{ pin, pkgs }: {
  demo = pin.github {
    target = "acme/demo";
    packages = {
      web = pin.pnpmPackage { root = "."; fetcherVersion = 4; };
      cli = pin.pnpmPackage { root = "."; fetcherVersion = 3; pnpm = pkgs.pnpm_10; };
    };
  };
}"#,
    );
    let path = project.0.join("pins.json");
    let mut pins: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    pins["pins"]["demo"]["sources"]["default"]["derived"] = json!({
        "web.pnpmDepsHash": "sha256-BlpIDik4hkU4c+KCyAmgUURIN362RDQID/qo6Ojp2Ek=",
        "cli.pnpmDepsHash": "sha256-GBccl8V87u26dtrGpHR+rKqRBqX6lq1SBwfsPvj/+44="
    });
    fs::write(&path, serde_json::to_vec(&pins).unwrap()).unwrap();
    let result = project.read(
        r#"let
  web = source.packages.web.pnpmDeps;
  cli = source.packages.cli.pnpmDeps;
in {
  webHash = web.outputHash;
  cliHash = cli.outputHash;
  usesPnpm10 = builtins.any (input: input.drvPath == pkgs.pnpm_10.drvPath) cli.nativeBuildInputs;
  distinct = web.drvPath != cli.drvPath;
}"#,
    );
    assert_eq!(result["webHash"], "sha256-BlpIDik4hkU4c+KCyAmgUURIN362RDQID/qo6Ojp2Ek=");
    assert_eq!(result["cliHash"], "sha256-GBccl8V87u26dtrGpHR+rKqRBqX6lq1SBwfsPvj/+44=");
    assert_eq!(result["usesPnpm10"], true);
    assert_eq!(result["distinct"], true);
    let status = Command::new(env!("CARGO_BIN_EXE_nix-pins"))
        .args(["status", "--config", "pins-config.nix", "--pins", "pins.json"])
        .current_dir(&project.0)
        .output()
        .unwrap();
    assert!(status.status.success(), "{}", String::from_utf8_lossy(&status.stderr));
}

#[test]
#[ignore = "需要 Nix 构建工具链和注册表依赖下载"]
fn update_produces_dependencies_for_a_real_offline_build() {
    let project = Project::new("{ pin }: {}");
    project.use_source(
        Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/pnpm-project")),
        ".",
        "",
    );
    fs::remove_file(project.0.join("pins.json")).unwrap();
    let output = project.update();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let pins: Value = serde_json::from_slice(&fs::read(project.0.join("pins.json")).unwrap()).unwrap();
    let hash = pins["pins"]["demo"]["sources"]["default"]["derived"]["pnpmDepsHash"]
        .as_str()
        .unwrap();
    assert!(hash.starts_with("sha256-"));
    let package = project.build(".");
    assert_eq!(fs::read_to_string(package.join("result.txt")).unwrap(), "true:false");
}

#[test]
#[ignore = "需要 Nix 构建工具链和注册表依赖下载"]
fn patched_workspace_in_a_subdirectory_builds_offline() {
    let project = Project::new("{ pin }: {}");
    project.use_source(
        Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/pnpm-workspace")),
        "web app",
        "cp 'web app/pnpm-lock.yaml.in' 'web app/pnpm-lock.yaml'",
    );
    fs::remove_file(project.0.join("pins.json")).unwrap();
    let output = project.update();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let package = project.build("web app");
    assert_eq!(
        fs::read_to_string(package.join("result.txt")).unwrap(),
        "workspace:true:false"
    );
}

#[test]
#[ignore = "需要 Nix 构建工具链和注册表依赖下载"]
fn pnpm_updates_reuse_inputs_and_preserve_failed_pins() {
    let project = Project::new("{ pin }: {}");
    project.use_source(
        Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/pnpm-project")),
        ".",
        "",
    );
    let path = project.0.join("pins.json");
    fs::remove_file(&path).unwrap();
    let first = project.update();
    assert!(first.status.success(), "{}", String::from_utf8_lossy(&first.stderr));
    let before = fs::read(&path).unwrap();
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    let repeated = project.update();
    assert!(
        repeated.status.success(),
        "{}",
        String::from_utf8_lossy(&repeated.stderr)
    );
    assert_eq!(fs::read(&path).unwrap(), before);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);

    let config_path = project.0.join("pins-config.nix");
    let config = fs::read_to_string(&config_path)
        .unwrap()
        .replace("{ pin }", "{ pin, pkgs }")
        .replace("fetcherVersion = 4;", "fetcherVersion = 3; pnpm = pkgs.pnpm_10;");
    fs::write(&config_path, &config).unwrap();
    let changed = project.update();
    assert!(changed.status.success(), "{}", String::from_utf8_lossy(&changed.stderr));
    let previous: Value = serde_json::from_slice(&before).unwrap();
    let updated: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let old_source = &previous["pins"]["demo"]["sources"]["default"];
    let new_source = &updated["pins"]["demo"]["sources"]["default"];
    assert_eq!(old_source["hash"], new_source["hash"]);
    assert_ne!(
        old_source["fingerprints"]["pnpmDepsHash"],
        new_source["fingerprints"]["pnpmDepsHash"]
    );
    assert_ne!(
        old_source["derived"]["pnpmDepsHash"],
        new_source["derived"]["pnpmDepsHash"]
    );

    let broken = config.replace(
        "postPatch = \"\";",
        r#"postPatch = ''substituteInPlace package.json --replace-fail '"7.0.0"' '"7.0.1"' '';"#,
    );
    let healthy = config.replace("demo = pin.mk", "healthy = pin.mk");
    fs::write(
        &config_path,
        format!("args@{{ pin, pkgs }}: ({broken}) args // ({healthy}) args"),
    )
    .unwrap();
    let failed = project.update();
    let error = String::from_utf8_lossy(&failed.stderr);
    assert!(!failed.status.success());
    assert!(error.contains("ERR_PNPM_OUTDATED_LOCKFILE"), "{error}");
    assert!(
        error.contains("Source 'default': Package 'default' Derived Hash 'pnpmDepsHash'"),
        "{error}"
    );
    let after: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(after["pins"]["demo"], updated["pins"]["demo"]);
    assert_eq!(after["pins"]["healthy"]["version"], "1.0.0");
    assert!(after["failures"]["demo"].as_str().unwrap().contains("pnpmDepsHash"));

    fs::remove_file(&path).unwrap();
    let failed = project.update();
    assert!(!failed.status.success());
    let initial: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert!(initial["pins"].get("demo").is_none());
    assert_eq!(initial["pins"]["healthy"]["version"], "1.0.0");
}
