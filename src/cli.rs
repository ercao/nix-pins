use clap::error::ErrorKind;
use clap::{Args, CommandFactory, Parser, Subcommand};
use regex::Regex;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "nix-pins", version, about = "Lock Nix package versions")]
struct Cli {
    /// 包含 Pin 声明的 Nix 配置。
    #[arg(long, global = true, value_name = "PATH", default_value = "./pins-config.nix")]
    config: PathBuf,
    /// 命令读取与更新的 Pins File。
    #[arg(long, global = true, value_name = "PATH", default_value = "./pins.json")]
    pins: PathBuf,
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
    pub config: PathBuf,
    pub pins: PathBuf,
    pub command: Command,
}

#[derive(Debug)]
pub struct Selection {
    names: Vec<String>,
    filter: Option<Regex>,
}

pub fn parse() -> Invocation {
    match try_parse_from(std::env::args_os()) {
        Ok(command) => command,
        Err(error) => error.exit(),
    }
}

impl Selection {
    pub fn matches(&self, name: &str) -> bool {
        (self.names.is_empty() && self.filter.is_none())
            || self.names.iter().any(|selected| selected == name)
            || self.filter.as_ref().is_some_and(|filter| filter.is_match(name))
    }

    pub fn validate<'a>(&self, names: impl IntoIterator<Item = &'a str>) -> Result<(), String> {
        let names: Vec<_> = names.into_iter().collect();
        let missing: Vec<_> = self
            .names
            .iter()
            .filter(|selected| !names.iter().any(|name| *name == selected.as_str()))
            .cloned()
            .collect();
        if !missing.is_empty() {
            return Err(format!("unknown pin name(s): {}", missing.join(", ")));
        }
        if self.names.is_empty() && self.filter.is_some() && !names.iter().any(|name| self.matches(name)) {
            return Err("filter matched no pins".into());
        }
        Ok(())
    }

    pub fn is_all(&self) -> bool {
        self.names.is_empty() && self.filter.is_none()
    }
}

fn try_parse_from<I, T>(args: I) -> Result<Invocation, clap::Error>
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    let cli = Cli::try_parse_from(args)?;
    let command = cli.command.unwrap_or(CliCommand::Status(SelectionArgs::default()));

    let command = match command {
        CliCommand::Update(selection) => selection.parse().map(Command::Update),
        CliCommand::Status(selection) => selection.parse().map(Command::Status),
    }?;
    Ok(Invocation {
        config: cli.config,
        pins: cli.pins,
        command,
    })
}

impl SelectionArgs {
    fn parse(self) -> Result<Selection, clap::Error> {
        let filter = self
            .filter
            .map(|pattern| {
                Regex::new(&pattern).map_err(|error| {
                    Cli::command().error(
                        ErrorKind::InvalidValue,
                        format!("invalid regular expression for '--filter <REGEX>': {error}"),
                    )
                })
            })
            .transpose()?;

        Ok(Selection {
            names: self.names,
            filter,
        })
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

        assert_eq!(invocation.config, std::path::PathBuf::from("config/custom.nix"));
        assert_eq!(invocation.pins, std::path::PathBuf::from("state/custom.json"));
        assert!(matches!(invocation.command, Command::Update(_)));
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
