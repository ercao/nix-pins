//! nix-pins —— Nix 包版本锁定工具。
//!
//! 子命令仅 update 与 status（ADR-0018），解析后交给应用模块执行。

#[cfg(not(unix))]
compile_error!("nix-pins only supports Unix platforms");

mod app;
mod checker;
mod cli;
mod nix;
mod pins;
mod probe;
mod process;
mod progress;
mod selection;
mod settings;

use std::process::ExitCode;

fn main() -> ExitCode {
    app::run(cli::parse())
}
