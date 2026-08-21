//! nix-pins —— Nix 包版本锁定工具。
//!
//! 子命令仅 update 与 status（ADR-0009）。

mod checker;
mod nix;
mod pins;
mod probe;

use regex::Regex;
use std::collections::BTreeMap;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::{mpsc, Arc, Mutex};

const DEFAULT_CONFIG: &str = "./pins-config.nix";
const DEFAULT_PINS: &str = "./pins.json";

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

struct Selection {
    names: Vec<String>,
    filter: Option<Regex>,
}

fn main() -> ExitCode {
    let (cmd, selection) = match parse_args() {
        Ok(parsed) => parsed,
        Err(error) => {
            eprintln!("nix-pins: {error}");
            return ExitCode::from(2);
        }
    };
    let config = PathBuf::from(DEFAULT_CONFIG);
    let pins_path = PathBuf::from(DEFAULT_PINS);

    match cmd.as_str() {
        "update" => run_update(&config, &pins_path, &selection),
        "status" => run_status(&config, &pins_path, &selection),
        other => {
            eprintln!("nix-pins: unknown subcommand {other} (expected update or status)");
            ExitCode::from(2)
        }
    }
}

fn parse_args() -> Result<(String, Selection), String> {
    let mut args = std::env::args().skip(1);
    let command = args.next().unwrap_or_else(|| "status".into());
    let mut names = Vec::new();
    let mut filter = None;
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--filter" => {
                let pattern = args.next().ok_or("--filter needs a regex")?;
                filter = Some(Regex::new(&pattern).map_err(|error| error.to_string())?);
            }
            option if option.starts_with('-') => {
                return Err(format!("unknown option {option}"));
            }
            name => names.push(name.to_string()),
        }
    }
    Ok((command, Selection { names, filter }))
}

/// 部分失败保留旧条目，以非零退出码结束（ADR-0007）。
fn run_update(config: &Path, pins_path: &Path, selection: &Selection) -> ExitCode {
    match update(config, pins_path, selection) {
        Ok(failures) if failures.is_empty() => ExitCode::SUCCESS,
        Ok(failures) => {
            eprintln!("nix-pins: update completed with failures:");
            for (name, error) in failures {
                eprintln!("  {name}: {error}");
            }
            ExitCode::from(1)
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
) -> Result<BTreeMap<String, String>, String> {
    let mut pins = pins::PinsFile::load(pins_path).map_err(|e| e.to_string())?;
    let checker_options = checker::Options::from_env()?;
    let checks = probe::probe_checks(&config.display().to_string()).map_err(nix_error)?;
    let selected: BTreeMap<_, _> = checks
        .into_iter()
        .filter(|(name, _)| selection.matches(name))
        .collect();
    let mut versions = BTreeMap::new();
    let mut failures = BTreeMap::new();
    for (name, result) in run_checkers(
        selected,
        checker_options,
        env_jobs("NIX_PINS_CHECKER_JOBS", 8),
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
        match probe::probe_drvs(&config, &versions, &BTreeMap::new()) {
            Ok(mut source_probes) => {
                let source_recalculations = count_source_recalculations(&pins, &source_probes);
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
                for (name, result) in run_sources(tasks, env_jobs("NIX_PINS_HASH_JOBS", 1)) {
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
                    eprintln!("{source_recalculations} hashes need recalculation");
                } else {
                    let successful_versions = sources
                        .iter()
                        .map(|(name, source)| (name.clone(), source.version.clone()))
                        .collect();
                    let source_hashes = sources
                        .iter()
                        .map(|(name, source)| (name.clone(), source.hash.clone()))
                        .collect();
                    match probe::probe_drvs(&config, &successful_versions, &source_hashes) {
                        Ok(mut derived_probes) => {
                            let recalculations = source_recalculations
                                + count_derived_recalculations(&pins, &derived_probes);
                            eprintln!("{recalculations} hashes need recalculation");
                            let tasks = sources
                                .into_iter()
                                .map(|(name, source)| DerivedTask {
                                    probe: derived_probes.remove(&name),
                                    name,
                                    source,
                                })
                                .collect();
                            for (name, result) in
                                run_derived(tasks, env_jobs("NIX_PINS_HASH_JOBS", 1))
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
                            let error = nix_error(error);
                            for name in sources.into_keys() {
                                failures.insert(name, error.clone());
                            }
                        }
                    }
                }
            }
            Err(error) => {
                let error = nix_error(error);
                for name in versions.into_keys() {
                    failures.insert(name, error.clone());
                }
            }
        }
    }
    pins.failures.extend(failures.clone());
    pins.save(pins_path)
        .map_err(|e| e.to_string())
        .map(|()| failures)
}

impl Selection {
    fn matches(&self, name: &str) -> bool {
        (self.names.is_empty() && self.filter.is_none())
            || self.names.iter().any(|selected| selected == name)
            || self
                .filter
                .as_ref()
                .is_some_and(|filter| filter.is_match(name))
    }
}

fn run_sources(
    tasks: Vec<SourceTask>,
    jobs: usize,
) -> BTreeMap<String, Result<SourceResult, String>> {
    parallel_map(
        tasks.into_iter().map(|task| (task.name.clone(), task)),
        jobs,
        resolve_source,
    )
}

fn resolve_source(task: SourceTask) -> Result<SourceResult, String> {
    let probe = task
        .probe
        .ok_or_else(|| format!("Probe did not return {}", task.name))?;
    let hash = task
        .previous
        .as_ref()
        .filter(|pin| pin.fingerprints.get("hash") == Some(&probe.src))
        .map(|pin| Ok(pin.hash.clone()))
        .unwrap_or_else(|| {
            eprintln!("hashing {}: src", task.name);
            nix::resolve_hash(&probe.src).map_err(nix_error)
        })?;
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
) -> BTreeMap<String, Result<pins::Pin, String>> {
    parallel_map(
        tasks.into_iter().map(|task| (task.name.clone(), task)),
        jobs,
        resolve_derived,
    )
}

fn resolve_derived(task: DerivedTask) -> Result<pins::Pin, String> {
    let probe = task
        .probe
        .ok_or_else(|| format!("Probe did not return {}", task.name))?;
    let mut derived = BTreeMap::new();
    let mut fingerprints = BTreeMap::from([("hash".into(), task.source.fingerprint)]);
    for (key, drv_path) in probe.derived {
        let value = task
            .source
            .previous
            .as_ref()
            .filter(|pin| pin.fingerprints.get(&key) == Some(&drv_path))
            .and_then(|pin| pin.derived.get(&key))
            .cloned()
            .map(Ok)
            .unwrap_or_else(|| {
                eprintln!("hashing {}: {key}", task.name);
                nix::resolve_hash(&drv_path).map_err(|error| hash_error(&key, error))
            })?;
        derived.insert(key.clone(), value);
        fingerprints.insert(key, drv_path);
    }
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
) -> BTreeMap<String, Result<String, String>> {
    parallel_map(selected, jobs, move |declaration| {
        checker::check(&declaration, &options)
    })
}

fn parallel_map<K, T, R, I, F>(tasks: I, jobs: usize, function: F) -> BTreeMap<K, R>
where
    K: Ord + Send + 'static,
    T: Send + 'static,
    R: Send + 'static,
    I: IntoIterator<Item = (K, T)>,
    F: Fn(T) -> R + Send + Sync + 'static,
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
            sender.send((key, function(task))).unwrap();
        }));
    }
    drop(sender);
    let results = receiver.into_iter().collect();
    for worker in workers {
        worker.join().unwrap();
    }
    results
}

fn count_source_recalculations(
    pins: &pins::PinsFile,
    probe_results: &BTreeMap<String, probe::ProbeResult>,
) -> usize {
    probe_results
        .iter()
        .map(|(name, probe)| {
            let previous = pins.pins.get(name);
            usize::from(previous.and_then(|pin| pin.fingerprints.get("hash")) != Some(&probe.src))
        })
        .sum()
}

fn count_derived_recalculations(
    pins: &pins::PinsFile,
    probe_results: &BTreeMap<String, probe::ProbeResult>,
) -> usize {
    probe_results
        .iter()
        .map(|(name, probe)| {
            let previous = pins.pins.get(name);
            probe
                .derived
                .iter()
                .filter(|(key, fingerprint)| {
                    previous.and_then(|pin| pin.fingerprints.get(*key)) != Some(*fingerprint)
                })
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
