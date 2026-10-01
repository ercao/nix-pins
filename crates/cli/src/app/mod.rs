//! 命令分派与应用执行边界；各 Pin 的处理由内部流水线组织。

mod pipeline;
mod status;
mod update;

use crate::{cli, nix, process};
use std::process::ExitCode;

pub fn run(invocation: cli::Invocation) -> ExitCode {
    match invocation.command {
        cli::Command::Update(selection) => update::run(&invocation.settings, &selection),
        cli::Command::Status(selection) => {
            status::run(&invocation.settings.config, &invocation.settings.file, &selection)
        }
    }
}

fn ensure_not_cancelled() -> Result<(), String> {
    if process::cancelled() {
        Err("Cancelled".into())
    } else {
        Ok(())
    }
}

fn nix_error(error: nix::Error) -> String {
    match error {
        nix::Error::NoGotLine(output) | nix::Error::Nix(output) => nix::sanitize_activity(&output),
    }
}

fn hash_error(key: &str, error: nix::Error) -> String {
    match error {
        nix::Error::NoGotLine(output) if key == "npmDepsHash" => format!(
            "{}\nnix-pins: npmDepsHash needs a lockfile; use postPatch to provide package-lock.json when the source archive omits it",
            nix::sanitize_activity(&output)
        ),
        error => nix_error(error),
    }
}
