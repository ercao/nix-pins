//! 更新事务、版本检查与 Pin 结果汇总，成功计算后统一保存 Pins File。

use super::{
    ensure_not_cancelled, nix_error,
    pipeline::{run_checkers, run_pins},
};
use crate::{checker, nix, pins, probe, process, progress, selection::Selection, settings};
use std::{collections::BTreeMap, process::ExitCode};

struct UpdateResult {
    processed: usize,
    failures: BTreeMap<String, String>,
}

/// 部分失败保留旧条目，以非零退出码结束（ADR-0002）。
pub(super) fn run(settings: &settings::Settings, selection: &Selection) -> ExitCode {
    let progress = match progress::Progress::stderr() {
        Ok(progress) => progress,
        Err(error) => {
            eprintln!("nix-pins: failed to install Ctrl+C handler: {error}");
            return ExitCode::from(1);
        }
    };
    let result = update(settings, selection, &progress);
    let terminal = progress.finish();
    if process::cancelled() {
        if terminal {
            eprintln!();
        }
        eprintln!("nix-pins: Cancelled");
        eprintln!("Interrupted");
        return ExitCode::from(130);
    }
    match result {
        Ok(UpdateResult { processed, failures }) => {
            let failed = failures.len();
            if failed > 0 {
                if terminal {
                    eprintln!();
                }
                eprintln!("Failures:");
                for (name, error) in failures {
                    eprintln!("  {name}:");
                    for line in error.lines() {
                        eprintln!("    {line}");
                    }
                }
            }
            if terminal {
                eprintln!();
            }
            eprintln!("Processed {processed} pins; {failed} failed");
            if failed == 0 {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            }
        }
        Err(error) => {
            if terminal {
                eprintln!();
            }
            eprintln!("nix-pins: {}", nix::sanitize_activity(&error));
            ExitCode::from(1)
        }
    }
}

/// 全部版本检查完成后启动各 Pin 的独立流水线，最后在同一事务中保存结果。
fn update(
    settings: &settings::Settings,
    selection: &Selection,
    progress: &progress::Progress,
) -> Result<UpdateResult, String> {
    let config = &settings.config;
    let pins_path = &settings.file;
    ensure_not_cancelled()?;
    progress.phase(progress::Phase::LoadingConfiguration);
    let mut transaction = pins::PinsFile::transaction(pins_path).map_err(|error| error.to_string())?;
    let pins = &mut transaction.pins;
    let checker_options = checker::Options::from_settings(settings)?;

    let checks = probe::probe_checks(&config.display().to_string()).map_err(nix_error)?;
    selection.validate(checks.keys().map(String::as_str))?;
    if selection.is_all() {
        // 只有全量更新才清除配置中已删除的 Pin，局部更新保留未选择的历史条目。
        pins.pins.retain(|name, _| checks.contains_key(name));
        pins.failures.retain(|name, _| checks.contains_key(name));
    }

    let selected: BTreeMap<_, _> = checks.into_iter().filter(|(name, _)| selection.matches(name)).collect();
    let processed = selected.len();
    progress.phase(progress::Phase::CheckingVersions);
    let reporter = progress.reporter();
    for name in selected.keys() {
        reporter.declare(name, pins.pins.get(name).map(|pin| pin.version.as_str()));
    }

    let mut versions = BTreeMap::new();
    let mut failures = BTreeMap::new();
    for (name, result) in run_checkers(selected, checker_options, settings.checker_jobs, reporter.clone()) {
        pins.failures.remove(&name);
        match result {
            Ok(version) => {
                versions.insert(name, version);
            }
            Err(error) => {
                failures.insert(name, error);
            }
        }
    }

    ensure_not_cancelled()?;
    if !versions.is_empty() {
        progress.phase(progress::Phase::ProcessingPins);
        for (name, result) in run_pins(
            &config.display().to_string(),
            &versions,
            &pins.pins,
            settings.download_jobs,
            settings.hash_jobs,
            &reporter,
        ) {
            match result {
                Ok(pin) => {
                    pins.pins.insert(name, pin);
                }
                Err(error) => {
                    failures.insert(name, error);
                }
            }
        }
    }

    // 取消时放弃内存中的结果，避免把不完整的一轮更新写入文件。
    ensure_not_cancelled()?;
    for error in failures.values_mut() {
        *error = nix::sanitize_activity(error);
    }
    pins.failures.extend(failures.clone());
    progress.phase(progress::Phase::WritingPinsFile);
    let save_result = transaction.save().map_err(|error| error.to_string());
    if save_result.is_ok() {
        progress.complete();
    }

    save_result.map(|()| UpdateResult { processed, failures })
}
