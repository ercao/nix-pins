//! 加载应用级配置，在启动 Nix 前验证并发参数，并避免调试输出泄漏凭证。

use config::{Config, Environment, File, FileFormat};
use serde::Deserialize;
use std::path::PathBuf;

/// 应用运行参数；Pin 的声明仍由独立的 Nix 配置提供。
#[derive(clap::Args, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// 包含 Pin 声明的 Nix 配置。
    #[arg(
        long,
        env = "NIX_PINS_CONFIG",
        global = true,
        value_name = "PATH",
        help = "Nix configuration containing pin declarations (default: ./pins-config.nix)"
    )]
    pub config: PathBuf,
    /// 命令读取与更新的 Pins File。
    #[arg(
        long = "pins",
        env = "NIX_PINS_FILE",
        global = true,
        value_name = "PATH",
        help = "Pins file to read and update (default: ./pins.json)"
    )]
    pub file: PathBuf,
    /// Maximum concurrent version checks (default: half available CPUs, at least 1).
    #[arg(long, env = "NIX_PINS_CHECKER_JOBS", global = true)]
    pub checker_jobs: usize,
    /// Maximum concurrent downloads (default: 2).
    #[arg(long, env = "NIX_PINS_DOWNLOAD_JOBS", global = true)]
    pub download_jobs: usize,
    /// Maximum concurrent Nix evaluations and builds (default: half available CPUs, at least 1).
    #[arg(long, env = "NIX_PINS_HASH_JOBS", global = true)]
    pub hash_jobs: usize,
    /// GitHub API base URL (default: https://api.github.com).
    #[arg(long, env = "NIX_PINS_GITHUB_API_BASE", global = true, value_name = "URL")]
    pub github_api_base: String,
    /// crates.io API base URL (default: https://crates.io).
    #[arg(long, env = "NIX_PINS_CRATES_API_BASE", global = true, value_name = "URL")]
    pub crates_api_base: String,
    /// PyPI API base URL (default: https://pypi.org).
    #[arg(long, env = "NIX_PINS_PYPI_API_BASE", global = true, value_name = "URL")]
    pub pypi_api_base: String,
    /// npm registry base URL (default: https://registry.npmjs.org).
    #[arg(long, env = "NIX_PINS_NPM_REGISTRY_BASE", global = true, value_name = "URL")]
    pub npm_registry_base: String,
    /// GitHub token (GITHUB_TOKEN is a fallback below nix-pins.toml).
    #[arg(
        long,
        env = "NIX_PINS_GITHUB_TOKEN",
        global = true,
        value_name = "TOKEN",
        hide_env_values = true
    )]
    pub github_token: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        let cpu_jobs = (std::thread::available_parallelism().map_or(1, usize::from) / 2).max(1);
        Self {
            config: "./pins-config.nix".into(),
            file: "./pins.json".into(),
            checker_jobs: cpu_jobs,
            download_jobs: 2,
            hash_jobs: cpu_jobs,
            github_api_base: "https://api.github.com".into(),
            crates_api_base: "https://crates.io".into(),
            pypi_api_base: "https://pypi.org".into(),
            npm_registry_base: "https://registry.npmjs.org".into(),
            github_token: None,
        }
    }
}

impl Settings {
    /// 按兼容环境变量、TOML、Clap 解析的环境变量与 CLI 覆盖，缺失字段由 Serde 补齐默认值。
    pub fn load(overrides: Config) -> Result<Self, String> {
        let load = || -> Result<Self, config::ConfigError> {
            Config::builder()
                .add_source(Environment::with_prefix("GITHUB").keep_prefix(true).ignore_empty(true))
                .add_source(File::new("nix-pins.toml", FileFormat::Toml).required(false))
                .add_source(overrides)
                .build()?
                .try_deserialize()
        };
        let settings = load().map_err(|error| format!("invalid application configuration: {error}"))?;
        for (name, jobs) in [
            ("checker_jobs", settings.checker_jobs),
            ("download_jobs", settings.download_jobs),
            ("hash_jobs", settings.hash_jobs),
        ] {
            if jobs == 0 {
                return Err(format!(
                    "invalid application configuration: {name} must be greater than 0"
                ));
            }
        }
        Ok(settings)
    }
}

impl std::fmt::Debug for Settings {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // 不使用派生 Debug，避免把 github_token 带入错误报告或诊断日志。
        formatter
            .debug_struct("Settings")
            .field("config", &self.config)
            .field("file", &self.file)
            .field("checker_jobs", &self.checker_jobs)
            .field("download_jobs", &self.download_jobs)
            .field("hash_jobs", &self.hash_jobs)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serde_fills_only_missing_fields_with_defaults() {
        let settings: Settings = Config::builder()
            .set_override("download_jobs", 7)
            .unwrap()
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap();
        let defaults = Settings::default();

        assert_eq!(settings.config, defaults.config);
        assert_eq!(settings.file, defaults.file);
        assert_eq!(settings.download_jobs, 7);
        assert_eq!(settings.checker_jobs, defaults.checker_jobs);
        assert_eq!(settings.hash_jobs, defaults.hash_jobs);
        assert!(settings.checker_jobs > 0);
        assert_eq!(defaults.download_jobs, 2);
        assert_eq!(settings.github_api_base, defaults.github_api_base);
        assert_eq!(settings.crates_api_base, defaults.crates_api_base);
        assert_eq!(settings.pypi_api_base, defaults.pypi_api_base);
        assert_eq!(settings.npm_registry_base, defaults.npm_registry_base);
        assert!(settings.github_token.is_none());
    }
}
