//! 只读报告当前锁定结果与失败信息。

use super::nix_error;
use crate::{nix, pins, probe, selection::Selection};
use std::{path::Path, process::ExitCode};

/// 只读取当前结果并验证选择范围；不会运行 Checker 或写入 Pins File。
pub(super) fn run(config: &Path, pins_path: &Path, selection: &Selection) -> ExitCode {
    let result = (|| {
        let pins = pins::PinsFile::load(pins_path).map_err(|e| e.to_string())?;
        let checks = probe::probe_checks(&config.display().to_string()).map_err(nix_error)?;
        selection.validate(checks.keys().map(String::as_str))?;
        for name in checks.keys().filter(|name| selection.matches(name)) {
            match pins.pins.get(name) {
                Some(pin) => print!("{name} {}", pin.version),
                None => print!("{name} (not pinned)"),
            }
            if let Some(error) = pins.failures.get(name) {
                print!(" [failed: {error}]");
            }
            println!();
        }
        Ok::<_, String>(())
    })();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("nix-pins: {}", nix::sanitize_activity(&error));
            ExitCode::from(1)
        }
    }
}
