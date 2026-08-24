//! 终端 Pin 进度。业务线程只发送结构化事件，renderer 独占动态 stderr 区域。

use indicatif::{MultiProgress, ProgressBar, ProgressDrawTarget, ProgressStyle};
use std::collections::BTreeMap;
use std::io::{self, IsTerminal};
use std::sync::mpsc;
use std::thread::{self, JoinHandle};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PinStep {
    Checking,
    VersionSelected,
    HashingSource,
    SourceReady,
    SourceReused,
    DerivedReused(String),
    HashingDerived(String),
    HashingPackages,
}

impl PinStep {
    fn label(&self) -> String {
        match self {
            Self::Checking => "Checking".into(),
            Self::VersionSelected => "Version Selected".into(),
            Self::HashingSource => "Hashing source".into(),
            Self::SourceReady => "Source Ready".into(),
            Self::SourceReused => "Source Reused".into(),
            Self::DerivedReused(key) => format!("Reused {key}"),
            Self::HashingDerived(key) => format!("Hashing {key}"),
            Self::HashingPackages => "Hashing packages".into(),
        }
    }
}

enum Event {
    PinDeclared {
        name: String,
        current: String,
    },
    TargetSelected {
        name: String,
        target: String,
    },
    PinStep {
        name: String,
        step: PinStep,
    },
    PinDetail {
        name: String,
        detail: Option<String>,
    },
    PinPaused(String),
    PackagesStarted {
        name: String,
        packages: Vec<String>,
    },
    PackageActive {
        name: String,
        package: String,
        key: String,
    },
    PackageDetail {
        name: String,
        package: String,
        detail: Option<String>,
    },
    PackageReady {
        name: String,
        package: String,
        key: String,
    },
    PackageReused {
        name: String,
        package: String,
        key: String,
    },
    PackageFailed {
        name: String,
        package: String,
        key: String,
    },
    PinDone {
        name: String,
        packages: Option<usize>,
    },
    PinFailed {
        name: String,
        location: String,
    },
    OperationStarted(String),
    OperationFinished(Option<bool>),
    Message(String),
    Finished(mpsc::Sender<()>),
}

pub struct Progress {
    sender: Option<mpsc::Sender<Event>>,
    renderer: Option<JoinHandle<()>>,
}

impl Progress {
    pub fn stderr() -> Self {
        let target = io::stderr().is_terminal().then(ProgressDrawTarget::stderr);
        Self::spawn(target)
    }

    fn spawn(target: Option<ProgressDrawTarget>) -> Self {
        let (sender, receiver) = mpsc::channel();
        let renderer = thread::Builder::new()
            .name("nix-pins-progress".into())
            .spawn(move || render(receiver, target))
            .ok();
        Self {
            sender: renderer.as_ref().map(|_| sender),
            renderer,
        }
    }

    pub fn reporter(&self) -> Reporter {
        Reporter {
            sender: self.sender.clone(),
        }
    }

    pub fn operation(&self, label: impl Into<String>) -> Operation {
        let reporter = self.reporter();
        reporter.send(Event::OperationStarted(label.into()));
        Operation {
            reporter,
            finished: false,
        }
    }

    pub fn message(&self, message: impl Into<String>) {
        self.reporter().send(Event::Message(message.into()));
    }

    pub fn finish(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        if let Some(sender) = self.sender.take() {
            let (done, wait) = mpsc::channel();
            if sender.send(Event::Finished(done)).is_ok() {
                let _ = wait.recv();
            }
        }
        if let Some(renderer) = self.renderer.take() {
            let _ = renderer.join();
        }
    }
}

impl Drop for Progress {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[derive(Clone)]
pub struct Reporter {
    sender: Option<mpsc::Sender<Event>>,
}

impl Reporter {
    pub fn declare(&self, name: &str, current: Option<&str>) {
        self.send(Event::PinDeclared {
            name: name.into(),
            current: current.unwrap_or("—").into(),
        });
    }

    pub fn target(&self, name: &str, target: &str) {
        self.send(Event::TargetSelected {
            name: name.into(),
            target: target.into(),
        });
    }

    pub fn step(&self, name: &str, step: PinStep) {
        self.send(Event::PinStep {
            name: name.into(),
            step,
        });
    }

    pub fn detail(&self, name: &str, detail: Option<String>) {
        self.send(Event::PinDetail {
            name: name.into(),
            detail,
        });
    }

    pub fn pause(&self, name: &str) {
        self.send(Event::PinPaused(name.into()));
    }

    pub fn packages(&self, name: &str, packages: Vec<String>) {
        self.send(Event::PackagesStarted {
            name: name.into(),
            packages,
        });
    }

    pub fn package_active(&self, name: &str, package: &str, key: &str) {
        self.send(Event::PackageActive {
            name: name.into(),
            package: package.into(),
            key: key.into(),
        });
    }

    pub fn package_detail(&self, name: &str, package: &str, detail: Option<String>) {
        self.send(Event::PackageDetail {
            name: name.into(),
            package: package.into(),
            detail,
        });
    }

    pub fn package_ready(&self, name: &str, package: &str, key: &str) {
        self.send(Event::PackageReady {
            name: name.into(),
            package: package.into(),
            key: key.into(),
        });
    }

    pub fn package_reused(&self, name: &str, package: &str, key: &str) {
        self.send(Event::PackageReused {
            name: name.into(),
            package: package.into(),
            key: key.into(),
        });
    }

    pub fn package_failed(&self, name: &str, package: &str, key: &str) {
        self.send(Event::PackageFailed {
            name: name.into(),
            package: package.into(),
            key: key.into(),
        });
    }

    pub fn done(&self, name: &str, packages: Option<usize>) {
        self.send(Event::PinDone {
            name: name.into(),
            packages,
        });
    }

    pub fn failed(&self, name: &str, location: impl Into<String>) {
        self.send(Event::PinFailed {
            name: name.into(),
            location: location.into(),
        });
    }

    fn send(&self, event: Event) -> bool {
        self.sender.as_ref().is_some_and(|sender| sender.send(event).is_ok())
    }
}

pub struct Operation {
    reporter: Reporter,
    finished: bool,
}

impl Operation {
    pub fn finish(mut self) {
        self.reporter.send(Event::OperationFinished(None));
        self.finished = true;
    }

    pub fn succeed(mut self) {
        self.reporter.send(Event::OperationFinished(Some(true)));
        self.finished = true;
    }

    pub fn fail(mut self) {
        self.reporter.send(Event::OperationFinished(Some(false)));
        self.finished = true;
    }
}

impl Drop for Operation {
    fn drop(&mut self) {
        if !self.finished {
            self.reporter.send(Event::OperationFinished(None));
        }
    }
}

#[derive(Clone)]
struct PinVersions {
    current: String,
    target: String,
}

struct ActivePin {
    bar: ProgressBar,
    step: PinStep,
    detail: Option<String>,
    packages: BTreeMap<String, PackageState>,
}

enum PackageState {
    Waiting,
    Active { key: String, detail: Option<String> },
    Ready(String),
    Reused(String),
    Failed(String),
}

struct Renderer {
    multi: MultiProgress,
    versions: BTreeMap<String, PinVersions>,
    active: BTreeMap<String, ActivePin>,
    operation: ProgressBar,
}

impl Renderer {
    fn new(target: ProgressDrawTarget) -> Self {
        let multi = MultiProgress::with_draw_target(target);
        let operation = ProgressBar::new_spinner();
        operation.set_style(spinner_style());
        let operation = multi.add(operation);
        operation.finish_and_clear();
        Self {
            multi,
            versions: BTreeMap::new(),
            active: BTreeMap::new(),
            operation,
        }
    }

    fn apply(&mut self, event: Event) -> bool {
        match event {
            Event::PinDeclared { name, current } => {
                self.versions.insert(
                    name,
                    PinVersions {
                        current,
                        target: "…".into(),
                    },
                );
            }
            Event::TargetSelected { name, target } => {
                self.versions
                    .entry(name.clone())
                    .or_insert_with(|| PinVersions {
                        current: "—".into(),
                        target: "…".into(),
                    })
                    .target = target;
                self.refresh(&name);
            }
            Event::PinStep { name, step } => {
                self.ensure_active(&name, step);
                self.refresh(&name);
            }
            Event::PinDetail { name, detail } => {
                if let Some(pin) = self.active.get_mut(&name) {
                    pin.detail = detail;
                }
                self.refresh(&name);
            }
            Event::PinPaused(name) => {
                if let Some(pin) = self.active.remove(&name) {
                    pin.bar.finish_and_clear();
                }
            }
            Event::PackagesStarted { name, packages } => {
                self.ensure_active(&name, PinStep::HashingPackages);
                if let Some(pin) = self.active.get_mut(&name) {
                    pin.packages = packages
                        .into_iter()
                        .map(|package| (package, PackageState::Waiting))
                        .collect();
                }
                self.refresh(&name);
            }
            Event::PackageActive { name, package, key } => {
                if let Some(pin) = self.active.get_mut(&name) {
                    pin.packages.insert(package, PackageState::Active { key, detail: None });
                }
                self.refresh(&name);
            }
            Event::PackageDetail { name, package, detail } => {
                if let Some(PackageState::Active { detail: current, .. }) = self
                    .active
                    .get_mut(&name)
                    .and_then(|pin| pin.packages.get_mut(&package))
                {
                    *current = detail;
                }
                self.refresh(&name);
            }
            Event::PackageReady { name, package, key } => {
                if let Some(pin) = self.active.get_mut(&name) {
                    pin.packages.insert(package, PackageState::Ready(key));
                }
                self.refresh(&name);
            }
            Event::PackageReused { name, package, key } => {
                if let Some(pin) = self.active.get_mut(&name) {
                    pin.packages.insert(package, PackageState::Reused(key));
                }
                self.refresh(&name);
            }
            Event::PackageFailed { name, package, key } => {
                if let Some(pin) = self.active.get_mut(&name) {
                    pin.packages.insert(package, PackageState::Failed(key));
                }
                self.refresh(&name);
            }
            Event::PinDone { name, packages } => {
                self.finish_pin(&name, true, packages.map(|count| format!("{count} packages")))
            }
            Event::PinFailed { name, location } => self.finish_pin(&name, false, Some(location)),
            Event::OperationStarted(label) => {
                self.clear_operation();
                self.operation.set_message(label);
                self.operation.reset();
                self.operation.tick();
            }
            Event::OperationFinished(status) => {
                let label = self.operation.message().to_string();
                self.clear_operation();
                if let Some(success) = status {
                    let symbol = if success { "✓" } else { "✗" };
                    let _ = self.multi.println(format!("{symbol} {label}"));
                }
            }
            Event::Message(message) => {
                let _ = self.multi.println(message);
            }
            Event::Finished(done) => {
                self.clear();
                let _ = done.send(());
                return false;
            }
        }
        true
    }

    fn ensure_active(&mut self, name: &str, step: PinStep) {
        if let Some(pin) = self.active.get_mut(name) {
            pin.step = step;
            pin.detail = None;
            return;
        }
        let bar = ProgressBar::new_spinner();
        bar.set_style(spinner_style());
        let bar = self.multi.add(bar);
        bar.tick();
        self.active.insert(
            name.into(),
            ActivePin {
                bar,
                step,
                detail: None,
                packages: BTreeMap::new(),
            },
        );
    }

    fn refresh(&self, name: &str) {
        let Some(pin) = self.active.get(name) else {
            return;
        };
        let versions = self.versions.get(name).cloned().unwrap_or(PinVersions {
            current: "—".into(),
            target: "…".into(),
        });
        pin.bar.set_message(render_active(name, &versions, pin));
        pin.bar.tick();
    }

    fn finish_pin(&mut self, name: &str, success: bool, detail: Option<String>) {
        if let Some(pin) = self.active.remove(name) {
            pin.bar.finish_and_clear();
        }
        let versions = self.versions.get(name).cloned().unwrap_or(PinVersions {
            current: "—".into(),
            target: "…".into(),
        });
        let symbol = if success { "✓" } else { "✗" };
        let step = if success { "Done" } else { "Failed" };
        let suffix = detail.map(|detail| format!(" · {detail}")).unwrap_or_default();
        let _ = self.multi.println(format!(
            "{symbol} {name} {} {} {step}{suffix}",
            versions.current, versions.target
        ));
    }

    fn clear_operation(&mut self) {
        self.operation.finish_and_clear();
    }

    fn clear(&mut self) {
        for (_, pin) in std::mem::take(&mut self.active) {
            pin.bar.finish_and_clear();
        }
        self.clear_operation();
        let _ = self.multi.clear();
    }
}

fn spinner_style() -> ProgressStyle {
    ProgressStyle::with_template("{spinner} {msg}")
        .unwrap()
        .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏ ")
}

fn render_active(name: &str, versions: &PinVersions, pin: &ActivePin) -> String {
    let detail = pin
        .detail
        .as_ref()
        .map(|detail| format!(" · {detail}"))
        .unwrap_or_default();
    let mut line = format!(
        "{name} {} {} {}{detail}",
        versions.current,
        versions.target,
        pin.step.label()
    );
    let len = pin.packages.len();
    for (index, (package, state)) in pin.packages.iter().enumerate() {
        let branch = if index + 1 == len { "└─" } else { "├─" };
        let status = match state {
            PackageState::Waiting => "· waiting".into(),
            PackageState::Active { key, detail } => format!(
                "⠋ Hashing {key}{}",
                detail.as_ref().map(|detail| format!(" · {detail}")).unwrap_or_default()
            ),
            PackageState::Ready(key) => format!("✓ {key} ready"),
            PackageState::Reused(key) => format!("↺ {key} reused"),
            PackageState::Failed(key) => format!("✗ Failed · {key}"),
        };
        line.push_str(&format!("\n  {branch} {package} {status}"));
    }
    line
}

fn render(receiver: mpsc::Receiver<Event>, target: Option<ProgressDrawTarget>) {
    if let Some(target) = target {
        let mut renderer = Renderer::new(target);
        for event in receiver {
            if !renderer.apply(event) {
                break;
            }
        }
    } else {
        render_plain(receiver);
    }
}

fn render_plain(receiver: mpsc::Receiver<Event>) {
    let mut versions = BTreeMap::<String, PinVersions>::new();
    let mut operation = None;
    for event in receiver {
        match event {
            Event::PinDeclared { name, current } => {
                versions.insert(
                    name,
                    PinVersions {
                        current,
                        target: "…".into(),
                    },
                );
            }
            Event::TargetSelected { name, target } => {
                versions
                    .entry(name)
                    .or_insert_with(|| PinVersions {
                        current: "—".into(),
                        target: "…".into(),
                    })
                    .target = target;
            }
            Event::PinStep { name, step } => {
                let version = versions.get(&name).cloned().unwrap_or(PinVersions {
                    current: "—".into(),
                    target: "…".into(),
                });
                eprintln!("· {name} {} {} {}", version.current, version.target, step.label());
            }
            Event::PackageActive { name, package, key } => eprintln!("· {name} {package} Hashing {key}"),
            Event::PackageReused { name, package, key } => eprintln!("· {name} {package} Reused {key}"),
            Event::PinDone { name, packages } => {
                let version = versions.get(&name).cloned().unwrap_or(PinVersions {
                    current: "—".into(),
                    target: "…".into(),
                });
                let suffix = packages.map(|count| format!(" · {count} packages")).unwrap_or_default();
                eprintln!("✓ {name} {} {} Done{suffix}", version.current, version.target);
            }
            Event::PinFailed { name, location } => {
                let version = versions.get(&name).cloned().unwrap_or(PinVersions {
                    current: "—".into(),
                    target: "…".into(),
                });
                eprintln!("✗ {name} {} {} Failed · {location}", version.current, version.target);
            }
            Event::OperationStarted(label) => {
                eprintln!("· {label}");
                operation = Some(label);
            }
            Event::Finished(done) => {
                let _ = done.send(());
                break;
            }
            Event::OperationFinished(Some(success)) => {
                if let Some(label) = operation.take() {
                    eprintln!("{} {label}", if success { "✓" } else { "✗" });
                }
            }
            Event::OperationFinished(None) => operation = None,
            Event::Message(message) => eprintln!("{message}"),
            Event::PinDetail { .. }
            | Event::PinPaused(_)
            | Event::PackagesStarted { .. }
            | Event::PackageDetail { .. }
            | Event::PackageReady { .. }
            | Event::PackageFailed { .. } => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use indicatif::InMemoryTerm;

    #[test]
    fn renderer_shows_pin_versions_and_leaves_done_as_static_output() {
        let term = InMemoryTerm::new(8, 100);
        let mut renderer = Renderer::new(ProgressDrawTarget::term_like(Box::new(term.clone())));

        renderer.apply(Event::PinDeclared {
            name: "ripgrep".into(),
            current: "14.1.0".into(),
        });
        renderer.apply(Event::TargetSelected {
            name: "ripgrep".into(),
            target: "14.1.1".into(),
        });
        renderer.apply(Event::PinStep {
            name: "ripgrep".into(),
            step: PinStep::HashingSource,
        });
        assert!(
            term.contents().contains("ripgrep 14.1.0 14.1.1 Hashing source"),
            "{}",
            term.contents()
        );

        renderer.apply(Event::PinPaused("ripgrep".into()));
        assert_eq!(term.contents(), "");

        renderer.apply(Event::PinDone {
            name: "ripgrep".into(),
            packages: None,
        });
        assert!(
            term.contents().contains("✓ ripgrep 14.1.0 14.1.1 Done"),
            "{}",
            term.contents()
        );
    }

    #[test]
    fn writing_operation_can_leave_a_static_result() {
        let term = InMemoryTerm::new(4, 80);
        let mut renderer = Renderer::new(ProgressDrawTarget::term_like(Box::new(term.clone())));

        renderer.apply(Event::OperationStarted("Writing pins.json".into()));
        renderer.apply(Event::OperationFinished(Some(false)));

        assert!(term.contents().contains("✗ Writing pins.json"), "{}", term.contents());
    }

    #[test]
    fn operation_bar_is_reused_between_stages() {
        let term = InMemoryTerm::new(4, 80);
        let mut renderer = Renderer::new(ProgressDrawTarget::term_like(Box::new(term.clone())));
        let operation = renderer.operation.clone();

        renderer.apply(Event::OperationStarted("Loading configuration".into()));
        renderer.apply(Event::OperationFinished(None));
        term.moves_since_last_check();
        renderer.apply(Event::OperationStarted("Resolving sources".into()));

        assert_eq!(operation.message(), "Resolving sources");
        assert!(!term.moves_since_last_check().contains("Loading configuration"));
    }

    #[test]
    fn renderer_prints_message_without_corrupting_active_pin() {
        let term = InMemoryTerm::new(4, 100);
        let mut renderer = Renderer::new(ProgressDrawTarget::term_like(Box::new(term.clone())));
        renderer.apply(Event::PinDeclared {
            name: "demo".into(),
            current: "v1".into(),
        });
        renderer.apply(Event::TargetSelected {
            name: "demo".into(),
            target: "v2".into(),
        });
        renderer.apply(Event::PinStep {
            name: "demo".into(),
            step: PinStep::HashingSource,
        });
        renderer.apply(Event::Message("1 hashes need recalculation".into()));

        let contents = term.contents();
        assert!(contents.contains("1 hashes need recalculation\n"), "{contents}");
        assert!(contents.contains("demo v1 v2 Hashing source"), "{contents}");
    }

    #[test]
    fn package_tree_exists_only_while_pin_is_active() {
        let term = InMemoryTerm::new(10, 100);
        let mut renderer = Renderer::new(ProgressDrawTarget::term_like(Box::new(term.clone())));
        renderer.apply(Event::PinDeclared {
            name: "demo".into(),
            current: "v1".into(),
        });
        renderer.apply(Event::TargetSelected {
            name: "demo".into(),
            target: "v2".into(),
        });
        renderer.apply(Event::PackagesStarted {
            name: "demo".into(),
            packages: vec!["api".into(), "web".into()],
        });
        renderer.apply(Event::PackageReused {
            name: "demo".into(),
            package: "api".into(),
            key: "vendorHash".into(),
        });
        renderer.apply(Event::PackageActive {
            name: "demo".into(),
            package: "web".into(),
            key: "npmDepsHash".into(),
        });
        let active = term.contents();
        assert!(active.contains("├─ api ↺ vendorHash reused"), "{active}");
        assert!(active.contains("└─ web ⠋ Hashing npmDepsHash"), "{active}");
        renderer.apply(Event::PinDone {
            name: "demo".into(),
            packages: Some(2),
        });
        let done = term.contents();
        assert!(done.contains("✓ demo v1 v2 Done · 2 packages"), "{done}");
        assert!(!done.contains("npmDepsHash"), "{done}");
    }
}
