use crate::nix::{NixProgress, NixProgressUnit};
use prodash::render::line::{self, StreamKind};
use prodash::tree::{root, Item, Root};
use prodash::unit::{self, display::Mode};
use std::collections::BTreeMap;
use std::io::{self, Write};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PinStep {
    Checking,
    VersionSelected,
    HashingSource,
    PatchingSource,
    SourceReady,
    SourceReused,
    DerivedReused(String),
    HashingDerived(String),
}

impl PinStep {
    fn label(&self) -> String {
        match self {
            Self::Checking => "Checking".into(),
            Self::VersionSelected => "Version Selected".into(),
            Self::HashingSource => "Hashing source".into(),
            Self::PatchingSource => "Applying patches".into(),
            Self::SourceReady => "Source Ready".into(),
            Self::SourceReused => "Source Reused".into(),
            Self::DerivedReused(key) => format!("Reused {key}"),
            Self::HashingDerived(key) => format!("Hashing {key}"),
        }
    }
}

struct Node {
    item: Item,
    base: String,
    step: Option<String>,
}

impl Node {
    fn new(item: Item, base: String) -> Self {
        item.set_name(&base);
        Self { item, base, step: None }
    }

    fn set_base(&mut self, base: String) {
        self.base = base;
        self.reset();
    }

    fn set_step(&mut self, step: impl Into<String>) {
        self.step = Some(step.into());
        self.reset();
    }

    fn set_detail(&mut self, detail: Option<NixProgress>) {
        let Some(detail) = detail else {
            self.reset();
            return;
        };
        match detail {
            NixProgress::Text(text) => {
                self.item.init(None, None);
                self.item.set_name(format!("{} · {text}", self.name()));
            }
            NixProgress::Counter {
                current,
                total,
                unit,
                label,
            } => {
                let unit = match unit {
                    NixProgressUnit::Bytes => {
                        unit::dynamic_and_mode(prodash::unit::Bytes, Mode::with_percentage().and_throughput())
                    }
                    NixProgressUnit::Objects => {
                        unit::label_and_mode("objects", Mode::with_percentage().and_throughput())
                    }
                };
                self.item.init(total.map(saturating_usize), Some(unit));
                self.item.set(saturating_usize(current));
                self.item.set_name(format!("{} · {label}", self.name()));
            }
        }
    }

    fn reset(&self) {
        self.item.init(None, None);
        self.item.set_name(self.name());
    }

    fn name(&self) -> String {
        match &self.step {
            Some(step) => format!("{} · {step}", self.base),
            None => self.base.clone(),
        }
    }
}

fn saturating_usize(value: u64) -> usize {
    usize::try_from(value).unwrap_or(usize::MAX)
}

struct Source {
    node: Node,
    packages_collapsed: bool,
    packages: BTreeMap<String, Node>,
}

struct Pin {
    node: Node,
    current: String,
    target: Option<String>,
    sources_collapsed: bool,
    sources: BTreeMap<String, Source>,
    packages_collapsed: bool,
    packages: BTreeMap<String, Node>,
}

struct State {
    root: Arc<Root>,
    pins: BTreeMap<String, Pin>,
}

impl State {
    fn new(root: Arc<Root>) -> Self {
        Self {
            root,
            pins: BTreeMap::new(),
        }
    }
}

pub struct Progress {
    state: Arc<Mutex<State>>,
    renderer: Option<line::JoinHandle>,
}

impl Progress {
    pub fn stderr() -> Self {
        let root: Arc<Root> = root::Options {
            message_buffer_capacity: 1024,
            ..Default::default()
        }
        .into();
        let options = line::Options {
            frames_per_second: 10.0,
            throughput: true,
            initial_delay: None,
            hide_cursor: false,
            ..line::Options::default().auto_configure(StreamKind::Stderr)
        };
        let output_is_terminal = options.output_is_terminal;
        let output: Box<dyn Write + Send> = if output_is_terminal {
            Box::new(io::stderr())
        } else {
            Box::new(NonTerminalStderr(io::stderr()))
        };
        let renderer = catch_unwind(AssertUnwindSafe(|| {
            line::render(output, Arc::downgrade(&root), options)
        }))
        .ok();
        Self {
            state: Arc::new(Mutex::new(State::new(root))),
            renderer,
        }
    }

    pub fn reporter(&self) -> Reporter {
        Reporter {
            state: Arc::clone(&self.state),
        }
    }

    pub fn operation(&self, label: impl Into<String>) -> Operation {
        let item = self.state.lock().ok().map(|state| state.root.add_child(label.into()));
        Operation { item }
    }

    pub fn message(&self, message: impl Into<String>) {
        if let Ok(state) = self.state.lock() {
            let mut item = state.root.add_child("nix-pins");
            item.info(message);
        }
    }

    pub fn finish(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        if let Some(renderer) = self.renderer.take() {
            renderer.shutdown_and_wait();
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
    state: Arc<Mutex<State>>,
}

impl Reporter {
    pub fn declare(&self, name: &str, current: Option<&str>) {
        self.with_state(|state| {
            let current = current.unwrap_or("—").to_owned();
            let base = format!("{name} {current}");
            let item = state.root.add_child(&base);
            state.pins.insert(
                name.into(),
                Pin {
                    node: Node::new(item, base),
                    current,
                    target: None,
                    sources_collapsed: false,
                    sources: BTreeMap::new(),
                    packages_collapsed: false,
                    packages: BTreeMap::new(),
                },
            );
        });
    }

    pub fn target(&self, name: &str, target: &str) {
        self.with_pin(name, |pin| {
            pin.target = Some(target.into());
            pin.node.set_base(format!("{name} {} → {target}", pin.current));
        });
    }

    pub fn step(&self, name: &str, step: PinStep) {
        self.with_pin(name, |pin| pin.node.set_step(step.label()));
    }

    pub fn detail(&self, name: &str, detail: Option<NixProgress>) {
        self.with_pin(name, |pin| pin.node.set_detail(detail));
    }

    pub fn pause(&self, name: &str) {
        self.detail(name, None);
    }

    pub fn sources(&self, name: &str, sources: Vec<String>) {
        self.with_pin(name, |pin| {
            pin.sources.clear();
            pin.sources_collapsed = is_default_only(&sources);
            if !pin.sources_collapsed {
                for source in sources {
                    let item = pin.node.item.add_child(&source);
                    pin.sources.insert(
                        source.clone(),
                        Source {
                            node: Node::new(item, source),
                            packages_collapsed: false,
                            packages: BTreeMap::new(),
                        },
                    );
                }
            }
        });
    }

    pub fn source_step(&self, name: &str, source: &str, step: PinStep) {
        self.with_pin(name, |pin| {
            if pin.sources_collapsed && source == "default" {
                pin.node.set_step(step.label());
            } else if let Some(source) = pin.sources.get_mut(source) {
                source.node.set_step(step.label());
            }
        });
    }

    pub fn source_detail(&self, name: &str, source: &str, detail: Option<NixProgress>) {
        self.with_pin(name, |pin| {
            if pin.sources_collapsed && source == "default" {
                pin.node.set_detail(detail);
            } else if let Some(source) = pin.sources.get_mut(source) {
                source.node.set_detail(detail);
            }
        });
    }

    pub fn source_done(&self, name: &str, source: &str) {
        self.with_pin(name, |pin| {
            pin.sources.remove(source);
        });
    }

    pub fn source_packages(&self, name: &str, source: &str, packages: Vec<String>) {
        self.with_pin(name, |pin| {
            if pin.sources_collapsed && source == "default" {
                replace_packages(&mut pin.node, &mut pin.packages, &mut pin.packages_collapsed, packages);
            } else if let Some(source) = pin.sources.get_mut(source) {
                replace_packages(
                    &mut source.node,
                    &mut source.packages,
                    &mut source.packages_collapsed,
                    packages,
                );
            }
        });
    }

    pub fn source_package_active(&self, name: &str, source: &str, package: &str, key: &str) {
        self.with_source_packages(name, source, |parent, packages, collapsed| {
            package_step(parent, packages, collapsed, package, format!("Hashing {key}"));
        });
    }

    pub fn source_package_detail(&self, name: &str, source: &str, package: &str, detail: Option<NixProgress>) {
        self.with_source_packages(name, source, |parent, packages, collapsed| {
            package_detail(parent, packages, collapsed, package, detail);
        });
    }

    pub fn source_package_ready(&self, name: &str, source: &str, package: &str, key: &str) {
        self.with_source_packages(name, source, |parent, packages, collapsed| {
            finish_package(parent, packages, collapsed, package, key);
        });
    }

    pub fn source_package_reused(&self, name: &str, source: &str, package: &str, key: &str) {
        self.with_source_packages(name, source, |parent, packages, collapsed| {
            finish_package(parent, packages, collapsed, package, key);
        });
    }

    pub fn source_package_failed(&self, name: &str, source: &str, package: &str, key: &str) {
        self.with_source_packages(name, source, |parent, packages, collapsed| {
            finish_package(parent, packages, collapsed, package, key);
        });
    }

    pub fn packages(&self, name: &str, packages: Vec<String>) {
        self.with_pin(name, |pin| {
            replace_packages(&mut pin.node, &mut pin.packages, &mut pin.packages_collapsed, packages);
        });
    }

    pub fn package_active(&self, name: &str, package: &str, key: &str) {
        self.with_pin(name, |pin| {
            package_step(
                &mut pin.node,
                &mut pin.packages,
                pin.packages_collapsed,
                package,
                format!("Hashing {key}"),
            );
        });
    }

    pub fn package_detail(&self, name: &str, package: &str, detail: Option<NixProgress>) {
        self.with_pin(name, |pin| {
            package_detail(
                &mut pin.node,
                &mut pin.packages,
                pin.packages_collapsed,
                package,
                detail,
            );
        });
    }

    pub fn package_ready(&self, name: &str, package: &str, key: &str) {
        self.finish_pin_package(name, package, key);
    }

    pub fn package_reused(&self, name: &str, package: &str, key: &str) {
        self.finish_pin_package(name, package, key);
    }

    pub fn package_failed(&self, name: &str, package: &str, key: &str) {
        self.finish_pin_package(name, package, key);
    }

    pub fn done(&self, name: &str, packages: Option<usize>) {
        self.with_state(|state| {
            let Some(mut pin) = state.pins.remove(name) else {
                return;
            };
            let packages = packages.map(|count| format!(" · {count} packages")).unwrap_or_default();
            pin.node.item.set_name(name);
            match pin.target.as_deref() {
                Some(target) if target != pin.current => {
                    pin.node.item.done(format!("{} → {target}{packages}", pin.current));
                }
                _ => pin.node.item.info(format!("unchanged at {}{packages}", pin.current)),
            }
        });
    }

    pub fn failed(&self, name: &str, location: impl Into<String>) {
        let location = location.into();
        self.with_state(|state| {
            if let Some(mut pin) = state.pins.remove(name) {
                pin.node.item.set_name(name);
                pin.node.item.fail(format!("Failed · {location}"));
            }
        });
    }

    fn finish_pin_package(&self, name: &str, package: &str, key: &str) {
        self.with_pin(name, |pin| {
            finish_package(&mut pin.node, &mut pin.packages, pin.packages_collapsed, package, key);
        });
    }

    fn with_source_packages(
        &self,
        name: &str,
        source: &str,
        update: impl FnOnce(&mut Node, &mut BTreeMap<String, Node>, bool),
    ) {
        self.with_pin(name, |pin| {
            if pin.sources_collapsed && source == "default" {
                update(&mut pin.node, &mut pin.packages, pin.packages_collapsed);
            } else if let Some(source) = pin.sources.get_mut(source) {
                update(&mut source.node, &mut source.packages, source.packages_collapsed);
            }
        });
    }

    fn with_pin(&self, name: &str, update: impl FnOnce(&mut Pin)) {
        self.with_state(|state| {
            if let Some(pin) = state.pins.get_mut(name) {
                update(pin);
            }
        });
    }

    fn with_state(&self, update: impl FnOnce(&mut State)) {
        if let Ok(mut state) = self.state.lock() {
            update(&mut state);
        }
    }
}

fn is_default_only(names: &[String]) -> bool {
    names.len() == 1 && names[0] == "default"
}

fn replace_packages(
    parent: &mut Node,
    packages: &mut BTreeMap<String, Node>,
    collapsed: &mut bool,
    names: Vec<String>,
) {
    packages.clear();
    *collapsed = is_default_only(&names);
    if !*collapsed {
        for name in names {
            let item = parent.item.add_child(&name);
            packages.insert(name.clone(), Node::new(item, name));
        }
    }
}

fn package_step(
    parent: &mut Node,
    packages: &mut BTreeMap<String, Node>,
    collapsed: bool,
    package: &str,
    step: String,
) {
    if collapsed && package == "default" {
        parent.set_step(step);
    } else if let Some(package) = packages.get_mut(package) {
        package.set_step(step);
    }
}

fn package_detail(
    parent: &mut Node,
    packages: &mut BTreeMap<String, Node>,
    collapsed: bool,
    package: &str,
    detail: Option<NixProgress>,
) {
    if collapsed && package == "default" {
        parent.set_detail(detail);
    } else if let Some(package) = packages.get_mut(package) {
        package.set_detail(detail);
    }
}

fn finish_package(
    parent: &mut Node,
    packages: &mut BTreeMap<String, Node>,
    collapsed: bool,
    package: &str,
    _key: &str,
) {
    if collapsed && package == "default" {
        parent.reset();
    } else {
        packages.remove(package);
    }
}

struct NonTerminalStderr(io::Stderr);

impl Write for NonTerminalStderr {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        if buffer == b"\x1b[2K\r" {
            Ok(buffer.len())
        } else {
            self.0.write(buffer)
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

pub struct Operation {
    item: Option<Item>,
}

impl Operation {
    pub fn succeed(mut self) {
        if let Some(mut item) = self.item.take() {
            item.done("done");
        }
    }

    pub fn fail(mut self) {
        if let Some(mut item) = self.item.take() {
            item.fail("failed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prodash::messages::MessageLevel;
    use prodash::Root as _;

    fn reporter() -> (Reporter, Arc<Root>) {
        let root: Arc<Root> = root::Options {
            message_buffer_capacity: 1024,
            ..Default::default()
        }
        .into();
        let reporter = Reporter {
            state: Arc::new(Mutex::new(State::new(Arc::clone(&root)))),
        };
        (reporter, root)
    }

    #[test]
    fn builds_tree_and_collapses_default_nodes() {
        let (reporter, root) = reporter();
        reporter.declare("demo", Some("v1"));
        reporter.sources("demo", vec!["default".into()]);
        reporter.source_packages("demo", "default", vec!["default".into()]);
        assert_eq!(root.num_tasks(), 1);

        reporter.sources("demo", vec!["api".into(), "web".into()]);
        reporter.source_packages("demo", "api", vec!["cli".into(), "server".into()]);
        assert_eq!(root.num_tasks(), 5);

        reporter.source_done("demo", "api");
        assert_eq!(root.num_tasks(), 2);
    }

    #[test]
    fn applies_structured_counter_and_removes_finished_package() {
        let (reporter, root) = reporter();
        reporter.declare("demo", Some("v1"));
        reporter.packages("demo", vec!["api".into()]);
        reporter.package_detail(
            "demo",
            "api",
            Some(NixProgress::Counter {
                current: 13,
                total: Some(31),
                unit: NixProgressUnit::Objects,
                label: "Git objects".into(),
            }),
        );

        let mut snapshot = Vec::new();
        root.sorted_snapshot(&mut snapshot);
        let package = snapshot
            .iter()
            .find(|(_, task)| task.name.contains("Git objects"))
            .unwrap();
        let progress = package.1.progress.as_ref().unwrap();
        assert_eq!(progress.step.load(std::sync::atomic::Ordering::SeqCst), 13);
        assert_eq!(progress.done_at, Some(31));

        reporter.package_ready("demo", "api", "vendorHash");
        assert_eq!(root.num_tasks(), 1);
    }

    #[test]
    fn pin_completion_is_persistent_and_removes_active_node() {
        let (reporter, root) = reporter();
        reporter.declare("demo", Some("v1"));
        reporter.target("demo", "v2");
        reporter.done("demo", None);

        assert_eq!(root.num_tasks(), 0);
        let mut messages = Vec::new();
        root.copy_messages(&mut messages);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].level, MessageLevel::Success);
        assert!(messages[0].message.contains("v1 → v2"));
    }

    #[test]
    fn uses_large_message_buffer() {
        let (_, root) = reporter();
        assert_eq!(root.messages_capacity(), 1024);
    }

    #[test]
    fn successful_operation_is_persistent() {
        let (_, root) = reporter();
        Operation {
            item: Some(root.add_child("Loading configuration")),
        }
        .succeed();

        assert_eq!(root.num_tasks(), 0);
        let mut messages = Vec::new();
        root.copy_messages(&mut messages);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].level, MessageLevel::Success);
    }
}
