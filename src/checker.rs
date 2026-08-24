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
    Git(GitChecker),
    #[serde(rename = "crate", alias = "crates")]
    Crate(String),
    Pypi(String),
    Npm(NpmChecker),
    Url(UrlChecker),
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UrlChecker {
    url: String,
    #[serde(alias = "pattern")]
    regex: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NpmChecker {
    name: String,
    #[serde(default = "default_dist_tag", rename = "distTag", alias = "dist_tag")]
    dist_tag: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub enum GitChecker {
    Url(String),
    Options(GitCheckerOptions),
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GitCheckerOptions {
    url: String,
    #[serde(default)]
    mode: GitMode,
    branch: Option<String>,
    #[serde(rename = "ref")]
    reference: Option<String>,
    include: Option<String>,
    exclude: Option<String>,
    #[serde(default)]
    sort: GitSort,
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
enum GitMode {
    #[default]
    Tag,
    Head,
    Branch,
    Ref,
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
enum GitSort {
    #[default]
    Semver,
    Lexicographic,
}

#[derive(Clone)]
pub struct Options {
    client: Client,
    github_api_base: String,
    crates_api_base: String,
    pypi_api_base: String,
    npm_registry_base: String,
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
            npm_registry_base: env_or("NIX_PINS_NPM_REGISTRY_BASE", "https://registry.npmjs.org"),
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
        Checker::Git(config) => git_version(config),
        Checker::Crate(name) => crates_version(name, options),
        Checker::Pypi(name) => pypi_version(name, options),
        Checker::Npm(config) => npm_version(config, options),
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
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut lines = stdout.lines();
    let version = lines.next().map(str::trim).unwrap_or_default();
    if lines.next().is_some() {
        return Err("cmd Checker must produce exactly one nonempty line".into());
    }
    nonempty(version.to_string())
}

fn github_version(repository: &str, options: &Options) -> Result<String, String> {
    let (owner, repo) = repository.split_once('/').ok_or("github Checker expects owner/repo")?;
    let url = format!(
        "{}/repos/{owner}/{repo}/releases/latest",
        options.github_api_base.trim_end_matches('/')
    );
    let response = send_with_retry(|| {
        let request = options.client.get(&url);
        if let Some(token) = &options.github_token {
            request.bearer_auth(token)
        } else {
            request
        }
    })?;
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

fn git_version(config: &GitChecker) -> Result<String, String> {
    let (url, mode, branch, reference, include, exclude, sort) = match config {
        GitChecker::Url(url) => (url.as_str(), GitMode::Tag, None, None, None, None, GitSort::Semver),
        GitChecker::Options(config) => (
            config.url.as_str(),
            config.mode,
            config.branch.as_deref(),
            config.reference.as_deref(),
            config.include.as_deref(),
            config.exclude.as_deref(),
            config.sort,
        ),
    };

    match mode {
        GitMode::Tag => {
            let output = git_remote_tags(url)?;
            let include = include.map(Regex::new).transpose().map_err(|error| error.to_string())?;
            let exclude = exclude.map(Regex::new).transpose().map_err(|error| error.to_string())?;
            output
                .lines()
                .filter_map(|line| line.split_once("refs/tags/").map(|(_, tag)| tag))
                .filter(|tag| include.as_ref().is_none_or(|pattern| pattern.is_match(tag)))
                .filter(|tag| exclude.as_ref().is_none_or(|pattern| !pattern.is_match(tag)))
                .max_by(|left, right| match sort {
                    GitSort::Semver => compare_tags(left, right),
                    GitSort::Lexicographic => left.cmp(right),
                })
                .map(str::to_string)
                .ok_or("git Checker found no matching tags".into())
        }
        GitMode::Head => git_remote_version(url, "HEAD"),
        GitMode::Branch => git_remote_version(
            url,
            &format!(
                "refs/heads/{}",
                branch.ok_or("git Checker branch mode requires branch")?
            ),
        ),
        GitMode::Ref => git_remote_version(url, reference.ok_or("git Checker ref mode requires ref")?),
    }
}

fn git_remote_version(url: &str, reference: &str) -> Result<String, String> {
    let peeled = format!("{reference}^{{}}");
    let output = Command::new("git")
        .env("GIT_TERMINAL_PROMPT", "0")
        .args(["ls-remote", url, reference, &peeled])
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned());
    }

    let output = String::from_utf8_lossy(&output.stdout);
    let find = |target: &str| {
        output.lines().find_map(|line| {
            let (commit, found_ref) = line.split_once('\t')?;
            (found_ref == target).then(|| commit.to_string())
        })
    };
    find(&peeled)
        .or_else(|| find(reference))
        .ok_or_else(|| format!("git Checker found no remote ref '{reference}'"))
}

fn git_remote_tags(url: &str) -> Result<String, String> {
    let output = Command::new("git")
        .env("GIT_TERMINAL_PROMPT", "0")
        .args(["ls-remote", "--tags", "--refs", url])
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned());
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn crates_version(name: &str, options: &Options) -> Result<String, String> {
    let url = format!("{}/api/v1/crates/{name}", options.crates_api_base.trim_end_matches('/'));
    let value = get_json(&options.client, &url)?;
    value
        .get("crate")
        .and_then(|value| value.get("newest_version").or_else(|| value.get("max_version")))
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or("crates.io response has no version".into())
}

fn pypi_version(name: &str, options: &Options) -> Result<String, String> {
    let url = format!("{}/pypi/{name}/json", options.pypi_api_base.trim_end_matches('/'));
    let value = get_json(&options.client, &url)?;
    value
        .get("info")
        .and_then(|value| value.get("version"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or("PyPI response has no version".into())
}

fn npm_version(config: &NpmChecker, options: &Options) -> Result<String, String> {
    let mut url =
        reqwest::Url::parse(options.npm_registry_base.trim_end_matches('/')).map_err(|error| error.to_string())?;
    url.path_segments_mut()
        .map_err(|_| "npm registry base URL cannot be a base")?
        .push(&config.name);
    let value = get_json(&options.client, url.as_str())?;
    value
        .get("dist-tags")
        .and_then(|tags| tags.get(&config.dist_tag))
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| format!("npm package '{}' has no dist-tag '{}'", config.name, config.dist_tag))
}

fn default_dist_tag() -> String {
    "latest".into()
}

fn url_version(url: &str, pattern: &str, options: &Options) -> Result<String, String> {
    let body = send_with_retry(|| options.client.get(url))?
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
    send_with_retry(|| client.get(url))?
        .error_for_status()
        .map_err(|error| error.to_string())?
        .json()
        .map_err(|error| error.to_string())
}

fn send_with_retry<F>(mut request: F) -> Result<reqwest::blocking::Response, String>
where
    F: FnMut() -> reqwest::blocking::RequestBuilder,
{
    let delays = [
        Duration::from_millis(250),
        Duration::from_millis(500),
        Duration::from_secs(1),
    ];
    let mut last_response = None;
    for attempt in 0..=delays.len() {
        match request().send() {
            Ok(response) if retryable_status(response.status()) && attempt < delays.len() => {
                let delay = retry_after(&response).unwrap_or(delays[attempt]);
                last_response = Some(response);
                std::thread::sleep(delay);
            }
            Ok(response) => return Ok(response),
            Err(error) if (error.is_connect() || error.is_timeout()) && attempt < delays.len() => {
                std::thread::sleep(delays[attempt]);
            }
            Err(error) => {
                return last_response.ok_or_else(|| error.to_string());
            }
        }
    }

    unreachable!()
}

fn retryable_status(status: reqwest::StatusCode) -> bool {
    matches!(status.as_u16(), 429 | 500 | 502 | 503 | 504)
}

fn retry_after(response: &reqwest::blocking::Response) -> Option<Duration> {
    response
        .headers()
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .parse::<u64>()
        .ok()
        .filter(|seconds| *seconds > 0)
        .map(|seconds| Duration::from_secs(seconds.min(30)))
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

    struct GitFixture {
        path: std::path::PathBuf,
        first: String,
        second: String,
    }

    impl GitFixture {
        fn new() -> Self {
            let id = format!(
                "nix-pins-git-checker-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            );
            let root = std::env::temp_dir().join(id);
            let work = root.join("work");
            let path = root.join("remote.git");
            std::fs::create_dir_all(&root).unwrap();
            git(None, &["init", "--bare", path.to_str().unwrap()]);
            git(None, &["init", work.to_str().unwrap()]);
            git(Some(&work), &["config", "user.email", "tests@example.com"]);
            git(Some(&work), &["config", "user.name", "nix-pins tests"]);
            git(Some(&work), &["config", "commit.gpgsign", "false"]);
            git(Some(&work), &["config", "tag.gpgSign", "false"]);
            std::fs::write(work.join("version"), "one").unwrap();
            git(Some(&work), &["add", "version"]);
            git(Some(&work), &["commit", "-m", "first"]);
            let first = git(Some(&work), &["rev-parse", "HEAD"]);
            git(Some(&work), &["branch", "-M", "main"]);
            git(Some(&work), &["remote", "add", "origin", path.to_str().unwrap()]);
            git(Some(&work), &["push", "origin", "main"]);
            git(
                None,
                &[
                    "--git-dir",
                    path.to_str().unwrap(),
                    "symbolic-ref",
                    "HEAD",
                    "refs/heads/main",
                ],
            );
            git(Some(&work), &["branch", "stable", &first]);
            git(Some(&work), &["push", "origin", "stable"]);
            std::fs::write(work.join("version"), "two").unwrap();
            git(Some(&work), &["commit", "-am", "second"]);
            let second = git(Some(&work), &["rev-parse", "HEAD"]);
            git(Some(&work), &["tag", "v1.9.0", &first]);
            git(Some(&work), &["tag", "v1.10.0", &second]);
            git(Some(&work), &["tag", "v2.0.0-beta", &second]);
            git(Some(&work), &["tag", "release-2", &first]);
            git(Some(&work), &["tag", "release-10", &second]);
            git(Some(&work), &["push", "origin", "main"]);
            git(Some(&work), &["push", "origin", "--tags"]);
            git(Some(&work), &["push", "origin", "HEAD:refs/custom/nightly"]);
            Self { path, first, second }
        }

        fn url(&self) -> String {
            self.path.to_string_lossy().into_owned()
        }
    }

    impl Drop for GitFixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(self.path.parent().unwrap());
        }
    }

    fn git(cwd: Option<&std::path::Path>, args: &[&str]) -> String {
        let mut command = std::process::Command::new("git");
        command.args(args);
        if let Some(cwd) = cwd {
            command.current_dir(cwd);
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    #[test]
    fn git_checker_resolves_head_branch_and_ref_to_commits() {
        let fixture = GitFixture::new();
        let options = super::Options::from_env().unwrap();
        git(
            None,
            &[
                "-c",
                "user.name=nix-pins tests",
                "-c",
                "user.email=tests@example.com",
                "--git-dir",
                fixture.path.to_str().unwrap(),
                "tag",
                "-a",
                "annotated",
                fixture.second.as_str(),
                "-m",
                "annotated",
            ],
        );
        let cases = [
            (
                serde_json::json!({"git": {"url": fixture.url(), "mode": "head"}}),
                fixture.second.as_str(),
            ),
            (
                serde_json::json!({
                    "git": {"url": fixture.url(), "mode": "branch", "branch": "stable"}
                }),
                fixture.first.as_str(),
            ),
            (
                serde_json::json!({
                    "git": {
                        "url": fixture.url(),
                        "mode": "ref",
                        "ref": "refs/custom/nightly"
                    }
                }),
                fixture.second.as_str(),
            ),
            (
                serde_json::json!({
                    "git": {
                        "url": fixture.url(),
                        "mode": "ref",
                        "ref": "refs/tags/annotated"
                    }
                }),
                fixture.second.as_str(),
            ),
        ];

        for (declaration, expected) in cases {
            let checker = serde_json::from_value::<Checker>(declaration).unwrap();
            assert_eq!(super::check(&checker, &options).unwrap(), expected);
        }

        let checker = serde_json::from_value::<Checker>(serde_json::json!({
            "git": {
                "url": fixture.url(),
                "include": "^missing$"
            }
        }))
        .unwrap();
        assert_eq!(
            super::check(&checker, &options).unwrap_err(),
            "git Checker found no matching tags"
        );
    }

    #[test]
    fn git_checker_filters_tags_and_selects_the_requested_sort_order() {
        let fixture = GitFixture::new();
        let options = super::Options::from_env().unwrap();
        let cases = [
            (
                serde_json::json!({
                    "git": {
                        "url": fixture.url(),
                        "include": "^v",
                        "exclude": "-",
                        "sort": "semver"
                    }
                }),
                "v1.10.0",
            ),
            (
                serde_json::json!({
                    "git": {
                        "url": fixture.url(),
                        "include": "^release-",
                        "sort": "lexicographic"
                    }
                }),
                "release-2",
            ),
        ];

        for (declaration, expected) in cases {
            let checker = serde_json::from_value::<Checker>(declaration).unwrap();
            assert_eq!(super::check(&checker, &options).unwrap(), expected);
        }
    }

    struct HttpFixture {
        base: String,
        requests: std::sync::Arc<std::sync::atomic::AtomicUsize>,
        paths: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
        thread: Option<std::thread::JoinHandle<()>>,
    }

    impl HttpFixture {
        fn new(responses: Vec<String>) -> Self {
            use std::io::{Read, Write};

            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let requests = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let paths = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
            let request_count = requests.clone();
            let request_paths = paths.clone();
            let thread = std::thread::spawn(move || {
                for response in responses {
                    let (mut stream, _) = listener.accept().unwrap();
                    let mut request = [0_u8; 4096];
                    let size = stream.read(&mut request).unwrap();
                    let request = String::from_utf8_lossy(&request[..size]);
                    let path = request
                        .lines()
                        .next()
                        .and_then(|line| line.split_whitespace().nth(1))
                        .unwrap()
                        .to_string();
                    request_paths.lock().unwrap().push(path);
                    request_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    stream.write_all(response.as_bytes()).unwrap();
                }
            });
            Self {
                base: format!("http://{address}"),
                requests,
                paths,
                thread: Some(thread),
            }
        }
    }

    impl Drop for HttpFixture {
        fn drop(&mut self) {
            if let Some(thread) = self.thread.take() {
                thread.join().unwrap();
            }
        }
    }

    fn response(status: &str, headers: &[(&str, &str)], body: &str) -> String {
        let mut response = format!(
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n",
            body.len()
        );
        for (name, value) in headers {
            response.push_str(&format!("{name}: {value}\r\n"));
        }
        response.push_str("\r\n");
        response.push_str(body);
        response
    }

    #[test]
    fn http_get_retries_transient_statuses() {
        let fixture = HttpFixture::new(vec![
            response("503 Service Unavailable", &[], "busy"),
            response("200 OK", &[("Content-Type", "application/json")], "{\"ok\":true}"),
        ]);
        let client = reqwest::blocking::Client::builder().build().unwrap();
        let value = super::get_json(&client, &fixture.base).unwrap();
        assert_eq!(value["ok"], true);
        assert_eq!(fixture.requests.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    #[test]
    fn npm_checker_reads_a_scoped_package_dist_tag() {
        let fixture = HttpFixture::new(vec![response(
            "200 OK",
            &[("Content-Type", "application/json")],
            "{\"dist-tags\":{\"next\":\"2.0.0\"}}",
        )]);
        let mut options = super::Options::from_env().unwrap();
        options.npm_registry_base = fixture.base.clone();
        let checker = serde_json::from_value::<Checker>(serde_json::json!({
            "npm": {"name": "@scope/pkg", "distTag": "next"}
        }))
        .unwrap();
        assert_eq!(super::check(&checker, &options).unwrap(), "2.0.0");
        assert!(fixture.paths.lock().unwrap()[0].contains("%2Fpkg"));
    }

    #[test]
    fn http_get_honors_retry_after_seconds() {
        let fixture = HttpFixture::new(vec![
            response("429 Too Many Requests", &[("Retry-After", "1")], "busy"),
            response("200 OK", &[("Content-Type", "application/json")], "{\"ok\":true}"),
        ]);
        let client = reqwest::blocking::Client::builder().build().unwrap();
        let started = std::time::Instant::now();
        super::get_json(&client, &fixture.base).unwrap();
        assert!(started.elapsed() >= std::time::Duration::from_secs(1));
    }

    #[test]
    fn http_get_does_not_retry_permanent_or_parse_errors() {
        for response in [
            response("404 Not Found", &[], "missing"),
            response("200 OK", &[("Content-Type", "application/json")], "not-json"),
        ] {
            let fixture = HttpFixture::new(vec![response]);
            let client = reqwest::blocking::Client::builder().build().unwrap();
            assert!(super::get_json(&client, &fixture.base).is_err());
            assert_eq!(fixture.requests.load(std::sync::atomic::Ordering::SeqCst), 1);
        }
    }

    #[test]
    fn git_string_checker_remains_the_default_tag_mode() {
        let fixture = GitFixture::new();
        let options = super::Options::from_env().unwrap();
        let checker = serde_json::from_value::<Checker>(serde_json::json!({
            "git": fixture.url()
        }))
        .unwrap();
        assert_eq!(super::check(&checker, &options).unwrap(), "v2.0.0-beta");
    }

    #[test]
    fn http_get_stops_after_three_retries() {
        let fixture = HttpFixture::new(
            (0..4)
                .map(|_| response("503 Service Unavailable", &[], "busy"))
                .collect(),
        );
        let client = reqwest::blocking::Client::builder().build().unwrap();
        assert!(super::get_json(&client, &fixture.base).is_err());
        assert_eq!(fixture.requests.load(std::sync::atomic::Ordering::SeqCst), 4);
    }

    #[test]
    fn npm_checker_defaults_to_latest_and_reports_a_missing_tag() {
        let latest = HttpFixture::new(vec![response(
            "200 OK",
            &[("Content-Type", "application/json")],
            "{\"dist-tags\":{\"latest\":\"1.2.3\"}}",
        )]);
        let mut options = super::Options::from_env().unwrap();
        options.npm_registry_base = latest.base.clone();
        let checker = serde_json::from_value::<Checker>(serde_json::json!({
            "npm": {"name": "demo"}
        }))
        .unwrap();
        assert_eq!(super::check(&checker, &options).unwrap(), "1.2.3");

        let missing = HttpFixture::new(vec![response(
            "200 OK",
            &[("Content-Type", "application/json")],
            "{\"dist-tags\":{}}",
        )]);
        options.npm_registry_base = missing.base.clone();
        let error = super::check(&checker, &options).unwrap_err();
        assert!(error.contains("latest"), "{error}");
    }

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

    #[test]
    fn command_checker_requires_exactly_one_nonempty_line() {
        assert_eq!(super::command_version("printf '  v1.2.3  \\n'").unwrap(), "v1.2.3");
        assert!(super::command_version("printf ''").unwrap_err().contains("empty"));
        assert!(super::command_version("printf 'v1\\nv2\\n'")
            .unwrap_err()
            .contains("exactly one"));
    }
}
