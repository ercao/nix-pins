//! 用 Clap 解析命令并提前验证选择条件，未指定子命令时采用只读的 status。

use crate::selection::Selection;
use clap::error::ErrorKind;
use clap::parser::ValueSource;
use clap::{Args, CommandFactory, FromArgMatches, Parser, Subcommand};
use regex::Regex;
use std::collections::BTreeMap;

#[derive(Parser)]
#[command(
    name = "nix-pins",
    version,
    about = "Lock Nix package versions",
    after_help = "Configuration: optional ./nix-pins.toml; CLI > NIX_PINS_* > TOML > defaults."
)]
struct Cli {
    #[command(flatten)]
    settings: crate::settings::Settings,
    #[command(subcommand)]
    command: Option<CliCommand>,
}

#[derive(Subcommand)]
enum CliCommand {
    /// Update selected pins and write pins.json.
    Update(SelectionArgs),
    /// Report selected pins without updating them.
    Status(SelectionArgs),
}

#[derive(Args, Default)]
struct SelectionArgs {
    /// Exact pin names to select.
    #[arg(value_name = "PIN")]
    names: Vec<String>,
    /// Also select pin names matching this regular expression.
    #[arg(long, value_name = "REGEX")]
    filter: Option<String>,
}

#[derive(Debug)]
pub enum Command {
    Update(Selection),
    Status(Selection),
}

#[derive(Debug)]
pub struct Invocation {
    pub settings: crate::settings::Settings,
    pub command: Command,
}

pub fn parse() -> Invocation {
    match try_parse_from(std::env::args_os()) {
        Ok(command) => command,
        Err(error) => error.exit(),
    }
}

fn try_parse_from<I, T>(args: I) -> Result<Invocation, clap::Error>
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    // 更新模式允许省略 Settings 的非 Option 字段，缺失值留给配置文件和 Serde 默认值。
    let mut cli = Cli::command_for_update().mut_args(|arg| {
        // 延续空环境变量视为未设置的约定，不修改进程环境或影响显式 CLI 值。
        if arg
            .get_env()
            .and_then(std::env::var_os)
            .is_some_and(|value| value.is_empty())
        {
            arg.env(None::<&str>)
        } else {
            arg
        }
    });
    let matches = cli.try_get_matches_from_mut(args)?;
    let command = if matches.subcommand_name().is_some() {
        CliCommand::from_arg_matches(&matches)?
    } else {
        CliCommand::Status(SelectionArgs::default())
    };

    let command = match command {
        CliCommand::Update(selection) => selection.parse().map(Command::Update),
        CliCommand::Status(selection) => selection.parse().map(Command::Status),
    }?;
    // 当前配置均为单值参数；字段 id 与 Serde 键一致，只合并 CLI 或环境变量明确提供的值。
    let overrides: BTreeMap<_, _> = cli
        .get_arguments()
        .filter(|arg| arg.is_global_set())
        .filter_map(|arg| {
            let key = arg.get_id().as_str();
            if !matches!(
                matches.value_source(key),
                Some(ValueSource::CommandLine | ValueSource::EnvVariable)
            ) {
                return None;
            }
            let value = matches.get_raw(key)?.next()?;
            Some((key, value.to_string_lossy()))
        })
        .collect();
    Ok(Invocation {
        settings: config::Config::try_from(&overrides)
            .map_err(|error| format!("invalid application configuration: {error}"))
            .and_then(crate::settings::Settings::load)
            .map_err(|error| Cli::command_for_update().error(ErrorKind::InvalidValue, error))?,
        command,
    })
}

impl SelectionArgs {
    fn parse(self) -> Result<Selection, clap::Error> {
        let filter = self
            .filter
            .map(|pattern| {
                Regex::new(&pattern).map_err(|error| {
                    Cli::command_for_update().error(
                        ErrorKind::InvalidValue,
                        format!("invalid regular expression for '--filter <REGEX>': {error}"),
                    )
                })
            })
            .transpose()?;

        Ok(Selection::new(self.names, filter))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_status_for_all_pins() {
        let Command::Status(selection) = try_parse_from(["nix-pins"]).unwrap().command else {
            panic!("expected status");
        };

        assert!(selection.matches("ripgrep"));
    }

    #[test]
    fn parses_both_subcommands() {
        assert!(matches!(
            try_parse_from(["nix-pins", "update"]).unwrap().command,
            Command::Update(_)
        ));
        assert!(matches!(
            try_parse_from(["nix-pins", "status"]).unwrap().command,
            Command::Status(_)
        ));
    }

    #[test]
    fn exact_names_and_filter_are_a_union() {
        let Command::Update(selection) = try_parse_from(["nix-pins", "update", "alpha", "--filter", "^beta$"])
            .unwrap()
            .command
        else {
            panic!("expected update");
        };

        assert!(selection.matches("alpha"));
        assert!(selection.matches("beta"));
        assert!(!selection.matches("gamma"));
    }

    #[test]
    fn rejects_invalid_regular_expressions() {
        let error = try_parse_from(["nix-pins", "update", "--filter", "["]).unwrap_err();

        assert_eq!(error.kind(), ErrorKind::InvalidValue);
        assert!(error.to_string().contains("invalid regular expression"));
    }

    #[test]
    fn rejects_unknown_arguments() {
        let error = try_parse_from(["nix-pins", "update", "--unknown"]).unwrap_err();

        assert_eq!(error.kind(), ErrorKind::UnknownArgument);
    }

    #[test]
    fn parses_global_config_and_pins_paths() {
        let invocation = try_parse_from([
            "nix-pins",
            "--config",
            "config/custom.nix",
            "update",
            "--pins",
            "state/custom.json",
        ])
        .unwrap();

        assert_eq!(
            invocation.settings.config,
            std::path::PathBuf::from("config/custom.nix")
        );
        assert_eq!(invocation.settings.file, std::path::PathBuf::from("state/custom.json"));
        assert!(matches!(invocation.command, Command::Update(_)));
    }

    #[test]
    fn parses_global_application_config_options() {
        let invocation = try_parse_from([
            "nix-pins",
            "--checker-jobs",
            "3",
            "status",
            "--download-jobs",
            "4",
            "--hash-jobs",
            "5",
            "--github-api-base",
            "https://github.example",
            "--crates-api-base",
            "https://crates.example",
            "--pypi-api-base",
            "https://pypi.example",
            "--npm-registry-base",
            "https://npm.example",
            "--github-token",
            "test-token",
        ])
        .unwrap();

        let settings = invocation.settings;
        assert_eq!(
            (settings.checker_jobs, settings.download_jobs, settings.hash_jobs),
            (3, 4, 5)
        );
        assert_eq!(settings.github_api_base, "https://github.example");
        assert_eq!(settings.crates_api_base, "https://crates.example");
        assert_eq!(settings.pypi_api_base, "https://pypi.example");
        assert_eq!(settings.npm_registry_base, "https://npm.example");
        assert_eq!(settings.github_token.as_deref(), Some("test-token"));
        assert!(!format!("{settings:?}").contains("test-token"));
    }

    #[test]
    fn strict_selection_rejects_unknown_names_and_empty_filters() {
        let Command::Update(named) = try_parse_from(["nix-pins", "update", "missing"]).unwrap().command else {
            panic!("expected update");
        };
        assert!(named.validate(["demo"]).unwrap_err().contains("missing"));

        let Command::Update(filtered) = try_parse_from(["nix-pins", "update", "--filter", "^missing$"])
            .unwrap()
            .command
        else {
            panic!("expected update");
        };
        assert!(filtered.validate(["demo"]).unwrap_err().contains("matched no pins"));
    }
}
