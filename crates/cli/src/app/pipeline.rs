//! 各 Pin 的源码与派生哈希流水线，维护下载和 Nix 工作的独立并发限制。

use super::{hash_error, nix_error};
use crate::{checker, nix, pins, probe, process, progress};
use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex, mpsc};

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

enum HashTask {
    SourceProbe(String),
    DerivedProbe(String, BTreeMap<String, SourceResult>),
    Derived(Box<DerivedTask>),
}

enum PipelineEvent {
    SourceProbe(String, Result<probe::ProbeResult, String>),
    Source(String, String, Result<SourceResult, TaskFailure>),
    DerivedProbe(
        String,
        BTreeMap<String, SourceResult>,
        Result<probe::ProbeResult, String>,
    ),
    Derived(String, String, Result<pins::Source, Vec<TaskFailure>>),
}

#[derive(Default)]
struct PendingPin {
    remaining: usize,
    sources: BTreeMap<String, SourceResult>,
    resolved: BTreeMap<String, pins::Source>,
    failures: Vec<TaskFailure>,
}

fn probe_pin(
    config: &str,
    name: &str,
    version: &str,
    hashes: BTreeMap<String, String>,
) -> Result<probe::ProbeResult, String> {
    let versions = BTreeMap::from([(name.to_owned(), version.to_owned())]);
    let hashes = BTreeMap::from([(name.to_owned(), hashes)]);
    probe::probe_drvs(config, &versions, &hashes)
        .map_err(nix_error)?
        .remove(name)
        .ok_or_else(|| format!("Probe did not return pin '{name}'"))
}

/// 下载使用独立并发池，Nix 求值与派生哈希共用哈希池；仅同一 Pin 内等待全部 Source。
pub(super) fn run_pins(
    config: &str,
    versions: &BTreeMap<String, String>,
    previous: &BTreeMap<String, pins::Pin>,
    download_jobs: usize,
    hash_jobs: usize,
    reporter: &progress::Reporter,
) -> BTreeMap<String, Result<pins::Pin, String>> {
    std::thread::scope(|scope| {
        let (events, receiver) = mpsc::channel();
        let source_events = events.clone();
        let downloads = worker_pool(scope, download_jobs, move |task: SourceTask| {
            let pin = task.pin.clone();
            let source = task.source.clone();
            let expanded = task.expanded;
            let result = resolve_source(task, reporter);
            if result.is_ok() {
                reporter.source_wait(&pin, &source, "Waiting to build");
            } else if expanded {
                reporter.source_failed(&pin, &source);
            }
            let result = result.map_err(|error| TaskFailure {
                error: format!("Source '{source}' Hashing source: {error}"),
                location: format!("Source {source}/Hashing source"),
            });
            let _ = source_events.send(PipelineEvent::Source(pin, source, result));
        });
        let hashes = worker_pool(scope, hash_jobs, move |task| {
            let event = match task {
                HashTask::SourceProbe(pin) => {
                    reporter.step(&pin, progress::PinStep::ResolvingSources);
                    let result = probe_pin(config, &pin, &versions[&pin], BTreeMap::new());
                    reporter.wait(&pin, "Waiting to download");
                    PipelineEvent::SourceProbe(pin, result)
                }
                HashTask::DerivedProbe(pin, sources) => {
                    reporter.step(&pin, progress::PinStep::ResolvingDerivedHashes);
                    let source_hashes = sources
                        .iter()
                        .map(|(name, source)| (name.clone(), source.hash.clone()))
                        .collect();
                    let result = probe_pin(config, &pin, &versions[&pin], source_hashes);
                    reporter.wait(&pin, "Waiting to build");
                    PipelineEvent::DerivedProbe(pin, sources, result)
                }
                HashTask::Derived(task) => {
                    let pin = task.pin.clone();
                    let source = task.source.clone();
                    let expanded = task.source_result.expanded;
                    let result = resolve_derived(*task, reporter);
                    if expanded {
                        if result.is_ok() {
                            reporter.source_done(&pin, &source);
                        } else {
                            reporter.source_failed(&pin, &source);
                        }
                    }
                    PipelineEvent::Derived(pin, source, result)
                }
            };
            let _ = events.send(event);
        });
        for pin in versions.keys() {
            let _ = hashes.send(HashTask::SourceProbe(pin.clone()));
        }
        let mut pending = BTreeMap::<String, PendingPin>::new();
        let mut results = BTreeMap::new();
        while results.len() < versions.len() && !process::cancelled() {
            let event = match receiver.recv_timeout(std::time::Duration::from_millis(50)) {
                Ok(event) => event,
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            };
            match event {
                PipelineEvent::SourceProbe(pin, result) => {
                    let probe = match result {
                        Ok(probe) if !probe.sources.is_empty() => probe,
                        result => {
                            let error = result
                                .err()
                                .unwrap_or_else(|| format!("Probe returned no sources for pin '{pin}'"));
                            reporter.failed(&pin, "Resolving sources");
                            results.insert(pin, Err(error));
                            continue;
                        }
                    };
                    let expanded = probe.sources.len() != 1 || !probe.sources.contains_key("default");
                    pending.insert(
                        pin.clone(),
                        PendingPin {
                            remaining: probe.sources.len(),
                            ..Default::default()
                        },
                    );
                    if expanded {
                        reporter.sources(&pin, probe.sources.keys().cloned().collect());
                    }
                    for (source, probe) in probe.sources {
                        let previous = previous.get(&pin).and_then(|pin| pin.sources.get(&source)).cloned();
                        let _ = downloads.send(SourceTask {
                            pin: pin.clone(),
                            source,
                            expanded,
                            probe: Some(probe),
                            previous,
                        });
                    }
                }
                PipelineEvent::Source(pin, source, result) => {
                    let work = pending.get_mut(&pin).unwrap();
                    work.remaining -= 1;
                    match result {
                        Ok(source_result) => {
                            work.sources.insert(source, source_result);
                        }
                        Err(error) => work.failures.push(error),
                    }
                    if work.remaining == 0 {
                        if work.failures.is_empty() {
                            let _ = hashes.send(HashTask::DerivedProbe(pin, std::mem::take(&mut work.sources)));
                        } else {
                            let work = pending.remove(&pin).unwrap();
                            results.insert(pin.clone(), finish_pin(&pin, &versions[&pin], work, reporter));
                        }
                    }
                }
                PipelineEvent::DerivedProbe(pin, sources, result) => {
                    let mut probe = match result {
                        Ok(probe) => probe,
                        Err(error) => {
                            reporter.failed(&pin, "Resolving derived hashes");
                            pending.remove(&pin);
                            results.insert(pin, Err(error));
                            continue;
                        }
                    };
                    pending.get_mut(&pin).unwrap().remaining = sources.len();
                    for (source, source_result) in sources {
                        let _ = hashes.send(HashTask::Derived(Box::new(DerivedTask {
                            pin: pin.clone(),
                            probe: probe.sources.remove(&source),
                            source,
                            source_result,
                        })));
                    }
                }
                PipelineEvent::Derived(pin, source, result) => {
                    let work = pending.get_mut(&pin).unwrap();
                    work.remaining -= 1;
                    match result {
                        Ok(source_result) => {
                            work.resolved.insert(source, source_result);
                        }
                        Err(errors) => work.failures.extend(errors),
                    }
                    if work.remaining == 0 {
                        let work = pending.remove(&pin).unwrap();
                        results.insert(pin.clone(), finish_pin(&pin, &versions[&pin], work, reporter));
                    }
                }
            }
        }
        drop(downloads);
        drop(hashes);
        results
    })
}

/// 任一 Source 或派生哈希失败都拒绝整个新 Pin，由调用方保留上一轮完整条目。
fn finish_pin(
    name: &str,
    version: &str,
    mut work: PendingPin,
    reporter: &progress::Reporter,
) -> Result<pins::Pin, String> {
    if work.failures.is_empty() {
        reporter.done(name, None);
        Ok(pins::Pin {
            version: version.into(),
            sources: work.resolved,
        })
    } else {
        work.failures.sort_by(|left, right| left.location.cmp(&right.location));
        reporter.failed(
            name,
            work.failures
                .iter()
                .map(|failure| failure.location.as_str())
                .collect::<Vec<_>>()
                .join(", "),
        );
        Err(work
            .failures
            .into_iter()
            .map(|failure| failure.error)
            .collect::<Vec<_>>()
            .join("\n"))
    }
}

/// 关闭所有任务发送端后，空闲 worker 退出；作用域结束前会等待在途任务收尾。
fn worker_pool<'scope, T: Send + 'scope>(
    scope: &'scope std::thread::Scope<'scope, '_>,
    jobs: usize,
    work: impl Fn(T) + Send + Sync + 'scope,
) -> mpsc::Sender<T> {
    let (sender, receiver) = mpsc::channel();
    let receiver = Arc::new(Mutex::new(receiver));
    let work = Arc::new(work);
    for _ in 0..jobs.max(1) {
        let receiver = receiver.clone();
        let work = work.clone();
        scope.spawn(move || {
            loop {
                let task = receiver.lock().unwrap().recv();
                let Ok(task) = task else { break };
                if process::cancelled() {
                    break;
                }
                work(task);
            }
        });
    }
    sender
}

/// 用带固定占位哈希的源码 drvPath 判定复用，版本相同并不保证 Fetcher 输入未变。
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

/// 先实现补丁后的源码，再按各中间 FOD 的 drvPath 独立判定派生哈希是否可复用。
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

pub(super) fn run_checkers(
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
                reporter.wait(name, "Waiting to download");
            }
            Err(_) => reporter.failed(name, "Checking"),
        }
        result
    })
}

/// 工作队列只在领取任务时加锁；结果按键排序，避免完成顺序影响后续处理。
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
        workers.push(std::thread::spawn(move || {
            loop {
                if process::cancelled() {
                    break;
                }
                let Some((key, task)) = queue.lock().unwrap().pop_front() else {
                    break;
                };
                let result = function(&key, task);
                sender.send((key, result)).unwrap();
            }
        }));
    }
    drop(sender);
    let results = receiver.into_iter().collect();
    for worker in workers {
        worker.join().unwrap();
    }
    results
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Barrier;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn stage_workers_run_in_parallel_without_exceeding_the_limit() {
        for jobs in [1, 2, 3] {
            let active = AtomicUsize::new(0);
            let peak = AtomicUsize::new(0);
            let barrier = Barrier::new(jobs);
            std::thread::scope(|scope| {
                let tasks = worker_pool(scope, jobs, |_: usize| {
                    let count = active.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(count, Ordering::SeqCst);
                    barrier.wait();
                    active.fetch_sub(1, Ordering::SeqCst);
                });
                for task in 0..jobs * 2 {
                    tasks.send(task).unwrap();
                }
                drop(tasks);
            });
            assert_eq!(peak.load(Ordering::SeqCst), jobs);
            assert_eq!(active.load(Ordering::SeqCst), 0);
        }
    }
}
