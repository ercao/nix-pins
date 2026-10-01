//! nix-pins —— Nix 包版本锁定工具。
//!
//! 子命令仅 update 与 status（ADR-0009）。

mod checker;
mod cli;
mod nix;
mod pins;
mod probe;
mod progress;

use cli::{Command, Selection};
use std::collections::BTreeMap;
use std::collections::VecDeque;
use std::path::Path;
use std::process::ExitCode;
use std::sync::{mpsc, Arc, Mutex};

struct SourceTask {
    pin: String,
    source: String,
    expanded: bool,
    probe: Option<probe::SourceProbeResult>,
    previous: Option<pins::Source>,
}

struct SourceResult {
    expanded: bool,
    fetcher: pins::Fetcher,
    hash: String,
    fingerprint: String,
    previous: Option<pins::Source>,
}

struct DerivedTask {
    pin: String,
    source: String,
    source_result: SourceResult,
    probe: Option<probe::SourceProbeResult>,
}

struct TaskFailure {
    error: String,
    location: String,
}

struct UpdateResult {
    processed: usize,
    failures: BTreeMap<String, String>,
}

fn main() -> ExitCode {
    let invocation = cli::parse();
    match invocation.command {
        Command::Update(selection) => run_update(&invocation.config, &invocation.pins, &selection),
        Command::Status(selection) => run_status(&invocation.config, &invocation.pins, &selection),
    }
}

/// 部分失败保留旧条目，以非零退出码结束（ADR-0007）。
fn run_update(config: &Path, pins_path: &Path, selection: &Selection) -> ExitCode {
    let progress = match progress::Progress::stderr() {
        Ok(progress) => progress,
        Err(error) => {
            eprintln!("nix-pins: 无法安装 Ctrl+C 处理器: {error}");
            return ExitCode::from(1);
        }
    };
    let result = update(config, pins_path, selection, &progress);
    let terminal = progress.finish();
    if progress::cancelled() {
        if terminal {
            eprintln!();
        }
        eprintln!("nix-pins: 已取消");
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
            eprintln!("nix-pins: {}", progress::sanitize_activity(&error));
            ExitCode::from(1)
        }
    }
}

fn run_status(config: &Path, pins_path: &Path, selection: &Selection) -> ExitCode {
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
            eprintln!("nix-pins: {}", progress::sanitize_activity(&error));
            ExitCode::from(1)
        }
    }
}

fn update(
    config: &Path,
    pins_path: &Path,
    selection: &Selection,
    progress: &progress::Progress,
) -> Result<UpdateResult, String> {
    ensure_not_cancelled()?;
    progress.phase(progress::Phase::LoadingConfiguration);
    let mut transaction = pins::PinsFile::transaction(pins_path).map_err(|error| error.to_string())?;
    let pins = &mut transaction.pins;
    let checker_options = checker::Options::from_env()?;

    let checks = probe::probe_checks(&config.display().to_string()).map_err(nix_error)?;
    selection.validate(checks.keys().map(String::as_str))?;
    if selection.is_all() {
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
    for (name, result) in run_checkers(
        selected,
        checker_options,
        env_jobs("NIX_PINS_CHECKER_JOBS", 8),
        reporter.clone(),
    ) {
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
        let config = config.display().to_string();
        progress.phase(progress::Phase::ResolvingSources);
        let empty_hashes = BTreeMap::<String, BTreeMap<String, String>>::new();
        let (mut source_probes, source_probe_failures) =
            probe_pins(&config, &versions, &empty_hashes, env_jobs("NIX_PINS_HASH_JOBS", 1));
        for (pin, error) in source_probe_failures {
            reporter.failed(&pin, "Resolving sources");
            failures.insert(pin, error);
        }
        {
            let mut expected_sources = BTreeMap::new();
            let mut tasks = Vec::new();
            for pin in versions.keys() {
                let Some(probe) = source_probes.remove(pin) else {
                    failures
                        .entry(pin.clone())
                        .or_insert_with(|| format!("Probe did not return pin '{pin}'"));
                    continue;
                };
                expected_sources.insert(pin.clone(), probe.sources.len());
                let expanded = probe.sources.len() != 1 || !probe.sources.contains_key("default");
                if expanded {
                    reporter.sources(pin, probe.sources.keys().cloned().collect());
                }
                for (source, source_probe) in probe.sources {
                    tasks.push(SourceTask {
                        previous: pins
                            .pins
                            .get(pin)
                            .and_then(|previous| previous.sources.get(&source))
                            .cloned(),
                        pin: pin.clone(),
                        source,
                        expanded,
                        probe: Some(source_probe),
                    });
                }
            }

            let mut sources = BTreeMap::<String, BTreeMap<String, SourceResult>>::new();
            let mut source_failures = BTreeMap::<String, Vec<TaskFailure>>::new();
            for ((pin, source), result) in run_sources(tasks, env_jobs("NIX_PINS_DOWNLOAD_JOBS", 8), reporter.clone()) {
                match result {
                    Ok(result) => {
                        sources.entry(pin).or_default().insert(source, result);
                    }
                    Err(failure) => source_failures.entry(pin).or_default().push(failure),
                }
            }
            for (pin, pin_failures) in source_failures {
                reporter.failed(
                    &pin,
                    pin_failures
                        .iter()
                        .map(|failure| failure.location.as_str())
                        .collect::<Vec<_>>()
                        .join(", "),
                );
                failures.insert(
                    pin,
                    pin_failures
                        .into_iter()
                        .map(|failure| failure.error)
                        .collect::<Vec<_>>()
                        .join("\n"),
                );
            }
            sources.retain(|pin, resolved| {
                !failures.contains_key(pin) && expected_sources.get(pin) == Some(&resolved.len())
            });

            ensure_not_cancelled()?;
            if !sources.is_empty() {
                let successful_versions = sources.keys().map(|pin| (pin.clone(), versions[pin].clone())).collect();
                let source_hashes = sources
                    .iter()
                    .map(|(pin, sources)| {
                        (
                            pin.clone(),
                            sources
                                .iter()
                                .map(|(source, result)| (source.clone(), result.hash.clone()))
                                .collect(),
                        )
                    })
                    .collect();
                progress.phase(progress::Phase::ResolvingDerivedHashes);
                let (mut derived_probes, derived_probe_failures) = probe_pins(
                    &config,
                    &successful_versions,
                    &source_hashes,
                    env_jobs("NIX_PINS_HASH_JOBS", 1),
                );
                ensure_not_cancelled()?;
                for (pin, error) in derived_probe_failures {
                    reporter.failed(&pin, "Resolving derived hashes");
                    failures.insert(pin, error);
                }
                {
                    let mut tasks = Vec::new();
                    for (pin, pin_sources) in sources {
                        if failures.contains_key(&pin) {
                            continue;
                        }
                        let mut source_probes = derived_probes
                            .remove(&pin)
                            .map(|probe| probe.sources)
                            .unwrap_or_default();
                        for (source, source_result) in pin_sources {
                            tasks.push(DerivedTask {
                                probe: source_probes.remove(&source),
                                pin: pin.clone(),
                                source,
                                source_result,
                            });
                        }
                    }

                    let mut resolved = BTreeMap::<String, BTreeMap<String, pins::Source>>::new();
                    let mut derived_failures = BTreeMap::<String, Vec<TaskFailure>>::new();
                    for ((pin, source), result) in
                        run_derived(tasks, env_jobs("NIX_PINS_HASH_JOBS", 1), reporter.clone())
                    {
                        match result {
                            Ok(result) => {
                                resolved.entry(pin).or_default().insert(source, result);
                            }
                            Err(task_failures) => derived_failures.entry(pin).or_default().extend(task_failures),
                        }
                    }
                    for (pin, pin_failures) in derived_failures {
                        reporter.failed(
                            &pin,
                            pin_failures
                                .iter()
                                .map(|failure| failure.location.as_str())
                                .collect::<Vec<_>>()
                                .join(", "),
                        );
                        failures.insert(
                            pin,
                            pin_failures
                                .into_iter()
                                .map(|failure| failure.error)
                                .collect::<Vec<_>>()
                                .join("\n"),
                        );
                    }

                    for (pin, sources) in resolved {
                        if failures.contains_key(&pin) || expected_sources.get(&pin) != Some(&sources.len()) {
                            continue;
                        }
                        pins.pins.insert(
                            pin.clone(),
                            pins::Pin {
                                version: versions[&pin].clone(),
                                sources,
                            },
                        );
                        reporter.done(&pin, None);
                    }
                }
            }
        }
    }

    ensure_not_cancelled()?;
    for error in failures.values_mut() {
        *error = progress::sanitize_activity(error);
    }
    pins.failures.extend(failures.clone());
    progress.phase(progress::Phase::WritingPinsFile);
    let save_result = transaction.save().map_err(|error| error.to_string());
    if save_result.is_ok() {
        progress.complete();
    }

    save_result.map(|()| UpdateResult { processed, failures })
}

fn ensure_not_cancelled() -> Result<(), String> {
    if progress::cancelled() {
        Err("已取消".into())
    } else {
        Ok(())
    }
}

fn probe_pins(
    config: &str,
    versions: &BTreeMap<String, String>,
    hashes: &BTreeMap<String, BTreeMap<String, String>>,
    jobs: usize,
) -> (BTreeMap<String, probe::ProbeResult>, BTreeMap<String, String>) {
    let config = config.to_owned();
    let hashes = hashes.clone();
    let tasks = versions
        .iter()
        .map(|(pin, version)| (pin.clone(), version.clone()))
        .collect::<BTreeMap<_, _>>();
    let results = parallel_map(tasks, jobs, move |pin, version| {
        let versions = BTreeMap::from([(pin.clone(), version)]);
        let hashes = hashes
            .get(pin)
            .map(|sources| BTreeMap::from([(pin.clone(), sources.clone())]))
            .unwrap_or_default();
        probe::probe_drvs(&config, &versions, &hashes)
            .map_err(nix_error)
            .and_then(|mut results| {
                results
                    .remove(pin)
                    .ok_or_else(|| format!("Probe did not return pin '{pin}'"))
            })
    });
    let mut probes = BTreeMap::new();
    let mut failures = BTreeMap::new();
    for (pin, result) in results {
        match result {
            Ok(probe) => {
                probes.insert(pin, probe);
            }
            Err(error) => {
                failures.insert(pin, error);
            }
        }
    }
    (probes, failures)
}

fn run_sources(
    tasks: Vec<SourceTask>,
    jobs: usize,
    reporter: progress::Reporter,
) -> BTreeMap<(String, String), Result<SourceResult, TaskFailure>> {
    parallel_map(
        tasks
            .into_iter()
            .map(|task| ((task.pin.clone(), task.source.clone()), task)),
        jobs,
        move |(pin, source), task| {
            let expanded = task.expanded;
            let result = resolve_source(task, &reporter);
            if result.is_ok() {
                if !expanded {
                    reporter.pause(pin);
                }
            } else if expanded {
                reporter.source_failed(pin, source);
            }
            result.map_err(|error| TaskFailure {
                error: format!("Source '{source}' Hashing source: {error}"),
                location: format!("Source {source}/Hashing source"),
            })
        },
    )
}

fn resolve_source(task: SourceTask, reporter: &progress::Reporter) -> Result<SourceResult, String> {
    let probe = task
        .probe
        .ok_or_else(|| format!("Probe did not return source '{}'", task.source))?;
    let fetcher = probe.fetcher.label().to_owned();
    let hash = if let Some(source) = task
        .previous
        .as_ref()
        .filter(|source| source.fingerprints.get("hash") == Some(&probe.src))
    {
        if task.expanded {
            reporter.source_step(&task.pin, &task.source, progress::PinStep::SourceReused);
            reporter.source_detail(&task.pin, &task.source, Some(fetcher.clone().into()));
        } else {
            reporter.step(&task.pin, progress::PinStep::SourceReused);
            reporter.detail(&task.pin, Some(fetcher.clone().into()));
        }
        source.hash.clone()
    } else {
        if task.expanded {
            reporter.source_step(&task.pin, &task.source, progress::PinStep::HashingSource);
            reporter.source_detail(&task.pin, &task.source, Some(fetcher.clone().into()));
        } else {
            reporter.step(&task.pin, progress::PinStep::HashingSource);
            reporter.detail(&task.pin, Some(fetcher.clone().into()));
        }
        let pin = task.pin.clone();
        let source = task.source.clone();
        let expanded = task.expanded;
        let fetcher_detail = fetcher.clone();
        let details = reporter.clone();
        let hash = nix::resolve_hash(&probe.src, move |detail| {
            let detail = Some(detail.map_or_else(
                || fetcher_detail.clone().into(),
                |detail| detail.with_prefix(&fetcher_detail),
            ));
            if expanded {
                details.source_detail(&pin, &source, detail);
            } else {
                details.detail(&pin, detail);
            }
        })
        .map_err(nix_error)?;
        if task.expanded {
            reporter.source_step(&task.pin, &task.source, progress::PinStep::SourceReady);
            reporter.source_detail(&task.pin, &task.source, Some(fetcher.clone().into()));
        } else {
            reporter.step(&task.pin, progress::PinStep::SourceReady);
            reporter.detail(&task.pin, Some(fetcher.clone().into()));
        }
        hash
    };

    Ok(SourceResult {
        expanded: task.expanded,
        fetcher: probe.fetcher,
        hash,
        fingerprint: probe.src,
        previous: task.previous,
    })
}

fn run_derived(
    tasks: Vec<DerivedTask>,
    jobs: usize,
    reporter: progress::Reporter,
) -> BTreeMap<(String, String), Result<pins::Source, Vec<TaskFailure>>> {
    parallel_map(
        tasks
            .into_iter()
            .map(|task| ((task.pin.clone(), task.source.clone()), task)),
        jobs,
        move |_, task| {
            let pin = task.pin.clone();
            let source = task.source.clone();
            let expanded = task.source_result.expanded;
            let result = resolve_derived(task, &reporter);
            if expanded {
                if result.is_ok() {
                    reporter.source_done(&pin, &source);
                } else {
                    reporter.source_failed(&pin, &source);
                }
            }
            result
        },
    )
}

fn resolve_derived(task: DerivedTask, reporter: &progress::Reporter) -> Result<pins::Source, Vec<TaskFailure>> {
    let Some(probe) = task.probe else {
        return Err(vec![TaskFailure {
            error: format!("Source '{}': Probe did not return source", task.source),
            location: format!("Source {}/Resolving derived hashes", task.source),
        }]);
    };

    if let Some(patched) = &probe.patched {
        if task.source_result.expanded {
            reporter.source_step(&task.pin, &task.source, progress::PinStep::PatchingSource);
            reporter.source_detail(&task.pin, &task.source, Some(task.source_result.fetcher.label().into()));
        } else {
            reporter.step(&task.pin, progress::PinStep::PatchingSource);
            reporter.detail(&task.pin, Some(task.source_result.fetcher.label().into()));
        }
        let pin = task.pin.clone();
        let source = task.source.clone();
        let expanded = task.source_result.expanded;
        let fetcher = task.source_result.fetcher.label().to_owned();
        let details = reporter.clone();
        if let Err(error) = nix::realize(patched, move |detail| {
            let detail = Some(detail.map_or_else(|| fetcher.clone().into(), |detail| detail.with_prefix(&fetcher)));
            if expanded {
                details.source_detail(&pin, &source, detail);
            } else {
                details.detail(&pin, detail);
            }
        }) {
            return Err(vec![TaskFailure {
                error: format!("Source '{}': Applying patches: {}", task.source, nix_error(error)),
                location: format!("Source {}/Applying patches", task.source),
            }]);
        }
    }

    let packages = probe.package_derived();
    let tree = !packages.is_empty() && (packages.len() != 1 || !packages.contains_key("default"));
    if tree {
        if task.source_result.expanded {
            reporter.source_packages(&task.pin, &task.source, packages.keys().cloned().collect());
        } else {
            reporter.packages(&task.pin, packages.keys().cloned().collect());
        }
    }

    let mut derived = BTreeMap::new();
    let mut fingerprints = BTreeMap::from([("hash".into(), task.source_result.fingerprint)]);
    let mut failures = Vec::new();
    for (package, hashes) in &packages {
        for (key, drv_path) in hashes {
            let package_prefix = format!("{package}.");
            let display_key = key.strip_prefix(&package_prefix).unwrap_or(key);
            let reused = task
                .source_result
                .previous
                .as_ref()
                .filter(|source| source.fingerprints.get(key) == Some(drv_path))
                .and_then(|source| source.derived.get(key))
                .cloned();
            let hash = if let Some(hash) = reused {
                if tree {
                    if task.source_result.expanded {
                        reporter.source_package_reused(&task.pin, &task.source, package, display_key);
                    } else {
                        reporter.package_reused(&task.pin, package, display_key);
                    }
                } else if task.source_result.expanded {
                    reporter.source_step(
                        &task.pin,
                        &task.source,
                        progress::PinStep::DerivedReused(display_key.into()),
                    );
                } else {
                    reporter.step(&task.pin, progress::PinStep::DerivedReused(display_key.into()));
                }
                hash
            } else {
                if tree {
                    if task.source_result.expanded {
                        reporter.source_package_active(&task.pin, &task.source, package, display_key);
                    } else {
                        reporter.package_active(&task.pin, package, display_key);
                    }
                } else if task.source_result.expanded {
                    reporter.source_step(
                        &task.pin,
                        &task.source,
                        progress::PinStep::HashingDerived(display_key.into()),
                    );
                } else {
                    reporter.step(&task.pin, progress::PinStep::HashingDerived(display_key.into()));
                }
                let pin = task.pin.clone();
                let source = task.source.clone();
                let expanded = task.source_result.expanded;
                let package = package.clone();
                let progress_package = package.clone();
                let details = reporter.clone();
                let result = nix::resolve_hash(drv_path, move |detail| {
                    if tree {
                        if expanded {
                            details.source_package_detail(&pin, &source, &progress_package, detail);
                        } else {
                            details.package_detail(&pin, &progress_package, detail);
                        }
                    } else if expanded {
                        details.source_detail(&pin, &source, detail);
                    } else {
                        details.detail(&pin, detail);
                    }
                })
                .map_err(|error| hash_error(key, error));
                match result {
                    Ok(hash) => {
                        if tree {
                            if task.source_result.expanded {
                                reporter.source_package_ready(&task.pin, &task.source, &package, display_key);
                            } else {
                                reporter.package_ready(&task.pin, &package, display_key);
                            }
                        }
                        hash
                    }
                    Err(error) => {
                        if tree {
                            if task.source_result.expanded {
                                reporter.source_package_failed(&task.pin, &task.source, &package, display_key);
                            } else {
                                reporter.package_failed(&task.pin, &package, display_key);
                            }
                        }
                        let location = if task.source_result.expanded && tree {
                            format!("Source {}/Package {package}/Derived Hash {display_key}", task.source)
                        } else if task.source_result.expanded {
                            format!("Source {}/Derived Hash {display_key}", task.source)
                        } else if tree {
                            format!("Package {package}/Derived Hash {display_key}")
                        } else {
                            display_key.into()
                        };
                        failures.push(TaskFailure {
                            error: format!(
                                "Source '{}': Package '{package}' Derived Hash '{display_key}': {error}",
                                task.source
                            ),
                            location,
                        });
                        continue;
                    }
                }
            };
            derived.insert(key.clone(), hash);
            fingerprints.insert(key.clone(), drv_path.clone());
        }
    }

    if failures.is_empty() {
        Ok(pins::Source {
            fetcher: task.source_result.fetcher,
            hash: task.source_result.hash,
            derived,
            fingerprints,
        })
    } else {
        Err(failures)
    }
}

fn run_checkers(
    selected: BTreeMap<String, checker::Checker>,
    options: checker::Options,
    jobs: usize,
    reporter: progress::Reporter,
) -> BTreeMap<String, Result<String, String>> {
    parallel_map(selected, jobs, move |name, declaration| {
        reporter.step(name, progress::PinStep::Checking);
        let result = checker::check(&declaration, &options);
        match &result {
            Ok(version) => {
                reporter.target(name, version);
                reporter.step(name, progress::PinStep::VersionSelected);
                reporter.pause(name);
            }
            Err(_) => reporter.failed(name, "Checking"),
        }
        result
    })
}

fn parallel_map<K, T, R, I, F>(tasks: I, jobs: usize, function: F) -> BTreeMap<K, R>
where
    K: Ord + Send + 'static,
    T: Send + 'static,
    R: Send + 'static,
    I: IntoIterator<Item = (K, T)>,
    F: Fn(&K, T) -> R + Send + Sync + 'static,
{
    let queue = VecDeque::from_iter(tasks);
    let len = queue.len();
    if len == 0 {
        return BTreeMap::new();
    }
    let queue = Arc::new(Mutex::new(queue));
    let function = Arc::new(function);
    let (sender, receiver) = mpsc::channel();
    let mut workers = Vec::new();
    for _ in 0..jobs.max(1).min(len) {
        let queue = Arc::clone(&queue);
        let function = Arc::clone(&function);
        let sender = sender.clone();
        workers.push(std::thread::spawn(move || loop {
            if progress::cancelled() {
                break;
            }
            let Some((key, task)) = queue.lock().unwrap().pop_front() else {
                break;
            };
            let result = function(&key, task);
            sender.send((key, result)).unwrap();
        }));
    }
    drop(sender);
    let results = receiver.into_iter().collect();
    for worker in workers {
        worker.join().unwrap();
    }
    results
}

fn env_jobs(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|jobs| *jobs > 0)
        .unwrap_or(default)
}

fn nix_error(error: nix::Error) -> String {
    match error {
        nix::Error::NoGotLine(output) | nix::Error::Nix(output) => progress::sanitize_activity(&output),
    }
}

fn hash_error(key: &str, error: nix::Error) -> String {
    match error {
        nix::Error::NoGotLine(output) if key == "npmDepsHash" => format!(
            "{}\nnix-pins: npmDepsHash needs a lockfile; use postPatch to provide package-lock.json when the source archive omits it",
            progress::sanitize_activity(&output)
        ),
        error => nix_error(error),
    }
}
