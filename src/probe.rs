//! 对用户 Nix 配置的两阶段求值（ADR-0013、ADR-0015）。
//!
//! 阶段一只读 check，此时 pins 可为空 —— Nix 的惰性保证 src 中对
//! pins.<name>.version 的引用不被求值。阶段二注入版本后读 drvPath。

use crate::checker::Checker;
use crate::nix::{self, FAKE};
use crate::pins::Fetcher;
use std::collections::BTreeMap;

#[derive(Debug, serde::Deserialize)]
pub struct ProbeResult {
    pub src: String,
    pub fetcher: Fetcher,
    /// derived hash 名 → 承载它的中间 FOD drvPath（ADR-0011）。
    pub derived: BTreeMap<String, String>,
}

/// 阶段一：列出各 Pin 的 checker 声明。pins 传空集。
pub fn probe_checks(config: &str) -> Result<BTreeMap<String, Checker>, nix::Error> {
    let config = config_path(config)?;
    let expr = format!(
        "let cfg = import {config} {{ pkgs = import <nixpkgs> {{}}; fake = \"{FAKE}\"; pins = {{}}; }}; \
         in builtins.mapAttrs (n: p: p.check) cfg"
    );
    let value = nix::eval_json(&expr)?;
    serde_json::from_value(value).map_err(|error| nix::Error::Nix(error.to_string()))
}

/// 阶段二：注入已知版本，取出 src、Fetcher 与各中间 FOD 的 drvPath。
pub fn probe_drvs(
    config: &str,
    versions: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, ProbeResult>, nix::Error> {
    let config = config_path(config)?;
    let pins: BTreeMap<_, _> = versions
        .iter()
        .map(|(name, version)| (name, serde_json::json!({ "version": version })))
        .collect();
    let pins_json =
        serde_json::to_string(&pins).map_err(|error| nix::Error::Nix(error.to_string()))?;
    let pins_nix =
        serde_json::to_string(&pins_json).map_err(|error| nix::Error::Nix(error.to_string()))?;
    let expr = format!(
        "let pkgs = import <nixpkgs> {{}}; \
             pins = builtins.fromJSON {pins_nix}; \
             all = import {config} {{ inherit pkgs pins; fake = \"{FAKE}\"; }}; \
             cfg = builtins.listToAttrs (map (name: {{ inherit name; value = all.${{name}}; }}) (builtins.attrNames pins)); \
         in builtins.mapAttrs (n: p: \
             let d = if p ? derive then p.derive p.src else null; in {{ \
               src = p.src.drvPath; \
               fetcher = if p.src ? owner && p.src ? repo then {{ github = {{ inherit (p.src) owner repo rev; }}; }} \
                 else if p.src ? rev then {{ git = {{ inherit (p.src) url rev; }}; }} \
                 else {{ url = {{ inherit (p.src) url; }}; }}; \
               derived = if d == null then {{}} else \
                 (if d ? goModules then {{ vendorHash = d.goModules.drvPath; }} else {{}}) // \
                 (if d ? npmDeps then {{ npmDepsHash = d.npmDeps.drvPath; }} else {{}}); \
             }}) cfg"
    );
    let value = nix::eval_json(&expr)?;
    serde_json::from_value(value).map_err(|error| nix::Error::Nix(error.to_string()))
}

fn config_path(config: &str) -> Result<String, nix::Error> {
    let path = std::fs::canonicalize(config).map_err(|error| nix::Error::Nix(error.to_string()))?;
    serde_json::to_string(&path.to_string_lossy())
        .map_err(|error| nix::Error::Nix(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pins::Fetcher;

    #[test]
    fn probes_real_github_go_and_npm_derivations() {
        let versions = BTreeMap::from([
            ("curlie".into(), "v1.8.2".into()),
            ("sloc".into(), "0.3.2".into()),
        ]);
        let config = concat!(env!("CARGO_MANIFEST_DIR"), "/example/pins-config.nix");

        let results = probe_drvs(config, &versions).unwrap();

        assert!(matches!(results["curlie"].fetcher, Fetcher::Github(_)));
        assert!(results["curlie"].derived.contains_key("vendorHash"));
        assert!(results["sloc"].derived.contains_key("npmDepsHash"));
    }
}
