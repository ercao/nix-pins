//! 上游版本检查。内置常用 Checker，外部命令保留为 Escape Hatch（ADR-0001）。

use regex::Regex;
use reqwest::blocking::Client;
use semver::Version;
use serde::Deserialize;
use serde_json::Value;
use std::cmp::Ordering;
use std::process::Command;
use std::time::Duration;

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Checker {
    Cmd(String),
    Github(String),
    Git(String),
    #[serde(rename = "crate", alias = "crates")]
    Crate(String),
    Pypi(String),
    Url(UrlChecker),
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UrlChecker {
    url: String,
    #[serde(alias = "pattern")]
    regex: String,
}

#[derive(Clone)]
pub struct Options {
    client: Client,
    github_api_base: String,
    crates_api_base: String,
    pypi_api_base: String,
    github_token: Option<String>,
}

impl Options {
    pub fn from_env() -> Result<Self, String> {
        let client = Client::builder()
            .user_agent("nix-pins/0.1")
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|error| error.to_string())?;
        Ok(Self {
            client,
            github_api_base: env_or("NIX_PINS_GITHUB_API_BASE", "https://api.github.com"),
            crates_api_base: env_or("NIX_PINS_CRATES_API_BASE", "https://crates.io"),
            pypi_api_base: env_or("NIX_PINS_PYPI_API_BASE", "https://pypi.org"),
            github_token: std::env::var("NIX_PINS_GITHUB_TOKEN")
                .or_else(|_| std::env::var("GITHUB_TOKEN"))
                .ok(),
        })
    }
}

pub fn check(checker: &Checker, options: &Options) -> Result<String, String> {
    match checker {
        Checker::Cmd(command) => command_version(command),
        Checker::Github(repository) => github_version(repository, options),
        Checker::Git(url) => git_version(url),
        Checker::Crate(name) => crates_version(name, options),
        Checker::Pypi(name) => pypi_version(name, options),
        Checker::Url(config) => url_version(&config.url, &config.regex, options),
    }
}

fn command_version(command: &str) -> Result<String, String> {
    let output = Command::new("sh")
        .args(["-c", command])
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned());
    }
    nonempty(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn github_version(repository: &str, options: &Options) -> Result<String, String> {
    let (owner, repo) = repository
        .split_once('/')
        .ok_or("github Checker expects owner/repo")?;
    let url = format!(
        "{}/repos/{owner}/{repo}/releases/latest",
        options.github_api_base.trim_end_matches('/')
    );
    let mut request = options.client.get(url);
    if let Some(token) = &options.github_token {
        request = request.bearer_auth(token);
    }
    let response = request.send().map_err(|error| error.to_string())?;
    let status = response.status();
    if status.as_u16() == 403 || status.as_u16() == 429 {
        return Err("GitHub rate limit exhausted; configure NIX_PINS_GITHUB_TOKEN".into());
    }
    let value = response
        .error_for_status()
        .map_err(|error| error.to_string())?
        .json::<Value>()
        .map_err(|error| error.to_string())?;
    value
        .get("tag_name")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or("GitHub response has no tag_name".into())
}

fn git_version(url: &str) -> Result<String, String> {
    let output = Command::new("git")
        .args(["ls-remote", "--tags", "--refs", url])
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned());
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.split_once("refs/tags/").map(|(_, tag)| tag))
        .max_by(|left, right| compare_tags(left, right))
        .map(str::to_string)
        .ok_or("git Checker found no tags".into())
}

fn crates_version(name: &str, options: &Options) -> Result<String, String> {
    let url = format!(
        "{}/api/v1/crates/{name}",
        options.crates_api_base.trim_end_matches('/')
    );
    let value = get_json(&options.client, &url)?;
    value
        .get("crate")
        .and_then(|value| {
            value
                .get("newest_version")
                .or_else(|| value.get("max_version"))
        })
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or("crates.io response has no version".into())
}

fn pypi_version(name: &str, options: &Options) -> Result<String, String> {
    let url = format!(
        "{}/pypi/{name}/json",
        options.pypi_api_base.trim_end_matches('/')
    );
    let value = get_json(&options.client, &url)?;
    value
        .get("info")
        .and_then(|value| value.get("version"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or("PyPI response has no version".into())
}

fn url_version(url: &str, pattern: &str, options: &Options) -> Result<String, String> {
    let body = options
        .client
        .get(url)
        .send()
        .map_err(|error| error.to_string())?
        .error_for_status()
        .map_err(|error| error.to_string())?
        .text()
        .map_err(|error| error.to_string())?;
    let captures = Regex::new(pattern)
        .map_err(|error| error.to_string())?
        .captures(&body)
        .ok_or("URL regex did not match")?;
    nonempty(
        captures
            .get(1)
            .or_else(|| captures.get(0))
            .unwrap()
            .as_str()
            .to_string(),
    )
}

fn get_json(client: &Client, url: &str) -> Result<Value, String> {
    client
        .get(url)
        .send()
        .map_err(|error| error.to_string())?
        .error_for_status()
        .map_err(|error| error.to_string())?
        .json()
        .map_err(|error| error.to_string())
}

fn compare_tags(left: &str, right: &str) -> Ordering {
    match (
        Version::parse(left.trim_start_matches('v')),
        Version::parse(right.trim_start_matches('v')),
    ) {
        (Ok(left), Ok(right)) => left.cmp(&right),
        _ => left.cmp(right),
    }
}

fn nonempty(value: String) -> Result<String, String> {
    if value.is_empty() {
        Err("Checker produced an empty version".into())
    } else {
        Ok(value)
    }
}

fn env_or(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.into())
}

#[cfg(test)]
mod tests {
    use super::{compare_tags, Checker};
    use serde_json::json;
    use std::cmp::Ordering;

    #[test]
    fn semantic_tags_sort_numerically() {
        assert_eq!(compare_tags("v1.10.0", "v1.9.0"), Ordering::Greater);
    }

    #[test]
    fn checker_declaration_has_exactly_one_kind() {
        let error = serde_json::from_value::<Checker>(json!({
            "cmd": "printf v1",
            "github": "owner/repo"
        }))
        .unwrap_err();

        assert!(error.to_string().contains("single key"), "{error}");
    }
}
