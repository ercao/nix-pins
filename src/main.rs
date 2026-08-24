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
    name: String,
    version: String,
    probe: Option<probe::ProbeResult>,
    previous: Option<pins::Pin>,
}

struct SourceResult {
    version: String,
    fetcher: pins::Fetcher,
    hash: String,
    fingerprint: String,
    previous: Option<pins::Pin>,
}

struct DerivedTask {
    name: String,
    source: SourceResult,
    probe: Option<probe::ProbeResult>,
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
    let progress = progress::Progress::stderr();
    let result = update(config, pins_path, selection, &progress);
    progress.finish();
    match result {
        Ok(UpdateResult { processed, failures }) => {
            let failed = failures.len();
            if failed > 0 {
                eprintln!("nix-pins: update completed with failures:");
                for (name, error) in failures {
                    eprintln!("  {name}: {error}");
                }
            }
            eprintln!("Processed {processed} pins; {failed} failed");
            if failed == 0 {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            }
        }
        Err(error) => {
            eprintln!("nix-pins: {error}");
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
            eprintln!("nix-pins: {error}");
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
    let mut transaction = pins::PinsFile::transaction(pins_path).map_err(|error| error.to_string())?;
    let pins = &mut transaction.pins;
    let checker_options = checker::Options::from_env()?;

    let loading = progress.operation("Loading configuration");
    let checks = match probe::probe_checks(&config.display().to_string()).map_err(nix_error) {
        Ok(checks) => {
            loading.finish();
            checks
        }
        Err(error) => {
            loading.fail();
            return Err(error);
        }
    };
    selection.validate(checks.keys().map(String::as_str))?;
    if selection.is_all() {
        pins.pins.retain(|name, _| checks.contains_key(name));
        pins.failures.retain(|name, _| checks.contains_key(name));
    }

    let selected: BTreeMap<_, _> = checks.into_iter().filter(|(name, _)| selection.matches(name)).collect();
    let processed = selected.len();
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

    if !versions.is_empty() {
        let config = config.display().to_string();
        let resolving = progress.operation(format!("Resolving sources · {} pins", versions.len()));
        let source_probes = probe::probe_drvs(&config, &versions, &BTreeMap::new());
        match source_probes {
            Ok(mut source_probes) => {
                resolving.finish();
                let source_recalculations = count_source_recalculations(pins, &source_probes);
                let tasks = versions
                    .into_iter()
                    .map(|(name, version)| SourceTask {
                        previous: pins.pins.get(&name).cloned(),
                        probe: source_probes.remove(&name),
                        name,
                        version,
                    })
                    .collect();
                let mut sources = BTreeMap::new();
                for (name, result) in run_sources(tasks, env_jobs("NIX_PINS_HASH_JOBS", 1), reporter.clone()) {
                    match result {
                        Ok(source) => {
                            sources.insert(name, source);
                        }
                        Err(error) => {
                            failures.insert(name, error);
                        }
                    }
                }

                if sources.is_empty() {
                    progress.message(format!("{source_recalculations} hashes need recalculation"));
                } else {
                    let successful_versions = sources
                        .iter()
                        .map(|(name, source)| (name.clone(), source.version.clone()))
                        .collect();
                    let source_hashes = sources
                        .iter()
                        .map(|(name, source)| (name.clone(), source.hash.clone()))
                        .collect();
                    let resolving = progress.operation(format!("Resolving derived hashes · {} pins", sources.len()));
                    let derived_probes = probe::probe_drvs(&config, &successful_versions, &source_hashes);
                    match derived_probes {
                        Ok(mut derived_probes) => {
                            resolving.finish();
                            let recalculations =
                                source_recalculations + count_derived_recalculations(pins, &derived_probes);
                            progress.message(format!("{recalculations} hashes need recalculation"));
                            let tasks = sources
                                .into_iter()
                                .map(|(name, source)| DerivedTask {
                                    probe: derived_probes.remove(&name),
                                    name,
                                    source,
                                })
                                .collect();
                            for (name, result) in
                                run_derived(tasks, env_jobs("NIX_PINS_HASH_JOBS", 1), reporter.clone())
                            {
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
                        Err(error) => {
                            resolving.fail();
                            return Err(nix_error(error));
                        }
                    }
                }
            }
            Err(error) => {
                resolving.fail();
                return Err(nix_error(error));
            }
        }
    }

    pins.failures.extend(failures.clone());
    let writing = progress.operation("Writing pins.json");
    let save_result = transaction.save().map_err(|error| error.to_string());
    if save_result.is_ok() {
        writing.succeed();
    } else {
        writing.fail();
    }
    save_result.map(|()| UpdateResult { processed, failures })
}

fn run_sources(
    tasks: Vec<SourceTask>,
    jobs: usize,
    reporter: progress::Reporter,
) -> BTreeMap<String, Result<SourceResult, String>> {
    parallel_map(
        tasks.into_iter().map(|task| (task.name.clone(), task)),
        jobs,
        move |name, task| {
            let result = resolve_source(task, &reporter);
            if result.is_err() {
                reporter.failed(name, "Hashing source");
            } else {
                reporter.pause(name);
            }
            result
        },
    )
}

fn resolve_source(task: SourceTask, reporter: &progress::Reporter) -> Result<SourceResult, String> {
    let probe = task
        .probe
        .ok_or_else(|| format!("Probe did not return {}", task.name))?;
    let hash = if let Some(pin) = task
        .previous
        .as_ref()
        .filter(|pin| pin.fingerprints.get("hash") == Some(&probe.src))
    {
        reporter.step(&task.name, progress::PinStep::SourceReused);
        pin.hash.clone()
    } else {
        reporter.step(&task.name, progress::PinStep::HashingSource);
        let name = task.name.clone();
        let details = reporter.clone();
        let hash = nix::resolve_hash(&probe.src, move |detail| {
            details.detail(&name, detail);
        })
        .map_err(nix_error)?;
        reporter.step(&task.name, progress::PinStep::SourceReady);
        hash
    };
    Ok(SourceResult {
        version: task.version,
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
) -> BTreeMap<String, Result<pins::Pin, String>> {
    parallel_map(
        tasks.into_iter().map(|task| (task.name.clone(), task)),
        jobs,
        move |_, task| resolve_derived(task, &reporter),
    )
}

fn resolve_derived(task: DerivedTask, reporter: &progress::Reporter) -> Result<pins::Pin, String> {
    let Some(probe) = task.probe else {
        reporter.failed(&task.name, "Resolving derived hashes");
        return Err(format!("Probe did not return {}", task.name));
    };
    let packages = probe.package_derived();
    let tree = !packages.is_empty() && (packages.len() != 1 || !packages.contains_key("default"));
    if tree {
        reporter.packages(&task.name, packages.keys().cloned().collect());
    }

    let mut derived = BTreeMap::new();
    let mut fingerprints = BTreeMap::from([("hash".into(), task.source.fingerprint)]);
    for (package, hashes) in &packages {
        for (key, drv_path) in hashes {
            let package_prefix = format!("{package}.");
            let display_key = key.strip_prefix(&package_prefix).unwrap_or(key);
            let reused = task
                .source
                .previous
                .as_ref()
                .filter(|pin| pin.fingerprints.get(key) == Some(drv_path))
                .and_then(|pin| pin.derived.get(key))
                .cloned();
            let hash = if let Some(hash) = reused {
                if tree {
                    reporter.package_reused(&task.name, package, display_key);
                } else {
                    reporter.step(&task.name, progress::PinStep::DerivedReused(display_key.into()));
                }
                hash
            } else {
                if tree {
                    reporter.package_active(&task.name, package, display_key);
                } else {
                    reporter.step(&task.name, progress::PinStep::HashingDerived(display_key.into()));
                }
                let name = task.name.clone();
                let package = package.clone();
                let progress_package = package.clone();
                let details = reporter.clone();
                let result = nix::resolve_hash(drv_path, move |detail| {
                    if tree {
                        details.package_detail(&name, &progress_package, detail);
                    } else {
                        details.detail(&name, detail);
                    }
                })
                .map_err(|error| hash_error(key, error));
                match result {
                    Ok(hash) => {
                        if tree {
                            reporter.package_ready(&task.name, &package, display_key);
                        }
                        hash
                    }
                    Err(error) => {
                        if tree {
                            reporter.package_failed(&task.name, &package, display_key);
                        }
                        let location = if tree {
                            format!("{package}/{display_key}")
                        } else {
                            display_key.into()
                        };
                        reporter.failed(&task.name, location);
                        return Err(error);
                    }
                }
            };
            derived.insert(key.clone(), hash);
            fingerprints.insert(key.clone(), drv_path.clone());
        }
    }
    reporter.done(&task.name, tree.then_some(packages.len()));
    Ok(pins::Pin {
        version: task.source.version,
        fetcher: task.source.fetcher,
        hash: task.source.hash,
        derived,
        fingerprints,
    })
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
    K: AsRef<str> + Ord + Send + 'static,
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

fn count_source_recalculations(pins: &pins::PinsFile, probe_results: &BTreeMap<String, probe::ProbeResult>) -> usize {
    probe_results
        .iter()
        .map(|(name, probe)| {
            let previous = pins.pins.get(name);
            usize::from(previous.and_then(|pin| pin.fingerprints.get("hash")) != Some(&probe.src))
        })
        .sum()
}

fn count_derived_recalculations(pins: &pins::PinsFile, probe_results: &BTreeMap<String, probe::ProbeResult>) -> usize {
    probe_results
        .iter()
        .map(|(name, probe)| {
            let previous = pins.pins.get(name);
            probe
                .package_derived()
                .values()
                .flat_map(|derived| derived.iter())
                .filter(|(key, fingerprint)| previous.and_then(|pin| pin.fingerprints.get(*key)) != Some(*fingerprint))
                .count()
        })
        .sum()
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
        nix::Error::NoGotLine(output) | nix::Error::Nix(output) => output,
    }
}

fn hash_error(key: &str, error: nix::Error) -> String {
    match error {
        nix::Error::NoGotLine(output) if key == "npmDepsHash" => format!(
            "{output}\nnix-pins: npmDepsHash needs a lockfile; use postPatch to provide package-lock.json when the source archive omits it"
        ),
        error => nix_error(error),
    }
}
