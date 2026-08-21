use clap::error::ErrorKind;
use clap::{Args, CommandFactory, Parser, Subcommand};
use regex::Regex;

#[derive(Parser)]
#[command(name = "nix-pins", version, about = "Lock Nix package versions")]
struct Cli {
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
pub struct Selection {
    names: Vec<String>,
    filter: Option<Regex>,
}

pub fn parse() -> Command {
    match try_parse_from(std::env::args_os()) {
        Ok(command) => command,
        Err(error) => error.exit(),
    }
}

impl Selection {
    pub fn matches(&self, name: &str) -> bool {
        (self.names.is_empty() && self.filter.is_none())
            || self.names.iter().any(|selected| selected == name)
            || self
                .filter
                .as_ref()
                .is_some_and(|filter| filter.is_match(name))
    }
}

fn try_parse_from<I, T>(args: I) -> Result<Command, clap::Error>
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    let cli = Cli::try_parse_from(args)?;
    let command = cli
        .command
        .unwrap_or(CliCommand::Status(SelectionArgs::default()));

    match command {
        CliCommand::Update(selection) => selection.parse().map(Command::Update),
        CliCommand::Status(selection) => selection.parse().map(Command::Status),
    }
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
        let Command::Status(selection) = try_parse_from(["nix-pins"]).unwrap() else {
            panic!("expected status");
        };

        assert!(selection.matches("ripgrep"));
    }

    #[test]
    fn parses_both_subcommands() {
        assert!(matches!(
            try_parse_from(["nix-pins", "update"]).unwrap(),
            Command::Update(_)
        ));
        assert!(matches!(
            try_parse_from(["nix-pins", "status"]).unwrap(),
            Command::Status(_)
        ));
    }

    #[test]
    fn exact_names_and_filter_are_a_union() {
        let Command::Update(selection) =
            try_parse_from(["nix-pins", "update", "alpha", "--filter", "^beta$"]).unwrap()
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
}
