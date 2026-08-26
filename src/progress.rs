use crate::nix::{NixProgress, NixProgressUnit};
use prodash::render::line::{self, StreamKind};
use prodash::tree::{root, Item, Root};
use prodash::unit::{self, display::Mode};
use std::collections::BTreeMap;
use std::io::{self, Write};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Arc, Mutex};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

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
            Self::VersionSelected => "Version selected".into(),
            Self::HashingSource => "Hashing source".into(),
            Self::PatchingSource => "Applying patches".into(),
            Self::SourceReady => "Source ready".into(),
            Self::SourceReused => "Source reused".into(),
            Self::DerivedReused(key) => format!("Reused {key}"),
            Self::HashingDerived(key) => format!("Hashing {key}"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    LoadingConfiguration,
    CheckingVersions,
    ResolvingSources,
    ResolvingDerivedHashes,
    WritingPinsFile,
    Done,
}

impl Phase {
    fn label(self) -> &'static str {
        match self {
            Self::LoadingConfiguration => "Loading configuration",
            Self::CheckingVersions => "Checking versions",
            Self::ResolvingSources => "Resolving sources",
            Self::ResolvingDerivedHashes => "Resolving derived hashes",
            Self::WritingPinsFile => "Writing pins.json",
            Self::Done => "done",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Status {
    Waiting,
    Active,
    Success,
    Failure,
}

impl Status {
    fn symbol(self) -> &'static str {
        match self {
            Self::Waiting => "⏸",
            Self::Active => "⏵",
            Self::Success => "✔",
            Self::Failure => "⚠",
        }
    }

    fn required(self) -> bool {
        matches!(self, Self::Active | Self::Failure)
    }
}

#[derive(Clone)]
struct Node {
    status: Status,
    step: Option<String>,
    detail: Option<NixProgress>,
}

impl Node {
    fn new() -> Self {
        Self {
            status: Status::Waiting,
            step: None,
            detail: None,
        }
    }

    fn set_step(&mut self, step: impl Into<String>) {
        self.status = Status::Active;
        self.step = Some(step.into());
    }

    fn set_detail(&mut self, detail: Option<NixProgress>) {
        if detail.is_some() {
            self.status = Status::Active;
        }
        self.detail = detail;
    }

    fn clear_activity(&mut self) {
        self.step = None;
        self.detail = None;
    }

    fn pause(&mut self) {
        self.status = Status::Waiting;
        self.clear_activity();
    }

    fn succeed(&mut self) {
        self.status = Status::Success;
        self.clear_activity();
    }

    fn fail(&mut self, step: Option<String>) {
        self.status = Status::Failure;
        self.step = step;
        self.detail = None;
    }
}

struct Source {
    node: Node,
    packages_collapsed: bool,
    packages: BTreeMap<String, Node>,
}

struct Pin {
    node: Node,
    current: Option<String>,
    target: Option<String>,
    sources_collapsed: bool,
    sources: BTreeMap<String, Source>,
    packages_collapsed: bool,
    packages: BTreeMap<String, Node>,
    completed_at: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum NodeKey {
    Pin(String),
    Source(String, String),
    PinPackage(String, String),
    SourcePackage(String, String, String),
}

#[derive(Clone)]
struct TreeNode {
    key: NodeKey,
    base: String,
    node: Node,
    children: Vec<TreeNode>,
}

#[derive(Clone)]
struct SharedWriter {
    inner: Arc<Mutex<Box<dyn Write + Send>>>,
}

impl SharedWriter {
    fn new(writer: Box<dyn Write + Send>) -> Self {
        Self {
            inner: Arc::new(Mutex::new(writer)),
        }
    }
}

impl Write for SharedWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.inner
            .lock()
            .map_err(|_| io::Error::other("progress writer lock poisoned"))?
            .write(buffer)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner
            .lock()
            .map_err(|_| io::Error::other("progress writer lock poisoned"))?
            .flush()
    }
}

struct State {
    root: Arc<Root>,
    output: SharedWriter,
    output_is_terminal: bool,
    terminal_dimensions: (u16, u16),
    auto_dimensions: bool,
    renderer_options: Option<line::Options>,
    renderer: Option<line::JoinHandle>,
    phase: Option<Phase>,
    pins: BTreeMap<String, Pin>,
    title: Option<Item>,
    items: BTreeMap<NodeKey, Item>,
    completion_order: u64,
}

impl State {
    fn new(
        root: Arc<Root>,
        output: SharedWriter,
        output_is_terminal: bool,
        terminal_dimensions: (u16, u16),
        auto_dimensions: bool,
        renderer_options: Option<line::Options>,
        renderer: Option<line::JoinHandle>,
    ) -> Self {
        Self {
            root,
            output,
            output_is_terminal,
            terminal_dimensions,
            auto_dimensions,
            renderer_options,
            renderer,
            phase: None,
            pins: BTreeMap::new(),
            title: None,
            items: BTreeMap::new(),
            completion_order: 0,
        }
    }

    fn set_phase(&mut self, phase: Phase) {
        if self.phase == Some(phase) {
            return;
        }
        if !self.output_is_terminal {
            if let Some(previous) = self.phase.filter(|phase| *phase != Phase::Done) {
                let _ = writeln!(self.output, "{} done", previous.label());
            }
        }
        self.phase = Some(phase);
        self.rebuild();
    }

    fn complete(&mut self) {
        if self.phase == Some(Phase::Done) {
            return;
        }
        if !self.output_is_terminal {
            if let Some(previous) = self.phase {
                let _ = writeln!(self.output, "{} done", previous.label());
            }
        }
        self.phase = Some(Phase::Done);
        self.rebuild();
    }

    fn refresh_dimensions(&mut self) -> bool {
        if self.auto_dimensions && self.output_is_terminal {
            let dimensions = line::Options::default()
                .auto_configure(StreamKind::Stderr)
                .terminal_dimensions;
            if dimensions != self.terminal_dimensions {
                self.terminal_dimensions = dimensions;
                return true;
            }
        }
        false
    }

    fn rebuild(&mut self) {
        if !self.output_is_terminal {
            return;
        }
        let resized = self.refresh_dimensions();
        self.items.clear();
        self.title.take();

        let trees = self.visible_trees();
        let title_name = self.title_name(trees.len());
        let mut title = self.root.add_child(title_name);
        let mut items = BTreeMap::new();
        for tree in &trees {
            materialize_tree(&mut title, tree, 1, self.terminal_dimensions.0, &mut items);
        }
        self.items = items;
        self.title = Some(title);
        if resized {
            self.restart_renderer();
        }
    }

    fn restart_renderer(&mut self) {
        let Some(mut options) = self.renderer_options.clone() else {
            return;
        };
        if let Some(renderer) = self.renderer.take() {
            renderer.shutdown_and_wait();
        }
        options.terminal_dimensions = self.terminal_dimensions;
        self.renderer = catch_unwind(AssertUnwindSafe(|| {
            line::render(self.output.clone(), Arc::downgrade(&self.root), options)
        }))
        .ok();
    }

    fn shutdown_renderer(&mut self) {
        self.renderer_options = None;
        if let Some(renderer) = self.renderer.take() {
            renderer.shutdown_and_wait();
        }
    }

    fn refresh_items(&mut self) {
        if !self.output_is_terminal {
            return;
        }
        let width = self.terminal_dimensions.0;
        let keys: Vec<_> = self.items.keys().cloned().collect();
        for key in keys {
            let Some((base, node, level)) = self.node_for_key(&key) else {
                continue;
            };
            if let Some(item) = self.items.get_mut(&key) {
                apply_node(item, &base, &node, level, width);
            }
        }
    }

    fn visible_trees(&self) -> Vec<TreeNode> {
        self.visible_pin_names()
            .into_iter()
            .filter_map(|name| self.pin_tree(&name))
            .collect()
    }

    fn visible_pin_names(&self) -> Vec<String> {
        let budget = if self.terminal_dimensions.1 == 0 {
            20
        } else {
            usize::from(self.terminal_dimensions.1).saturating_div(3).max(1)
        };
        let mut remaining = budget.saturating_sub(1);
        let mut visible = Vec::new();

        for (name, pin) in &self.pins {
            if pin.node.status.required() {
                visible.push(name.clone());
                remaining = remaining.saturating_sub(pin_line_count(pin));
            }
        }

        for (name, pin) in &self.pins {
            if pin.node.status == Status::Waiting {
                let lines = pin_line_count(pin);
                if lines <= remaining {
                    visible.push(name.clone());
                    remaining -= lines;
                }
            }
        }

        let mut completed: Vec<_> = self
            .pins
            .iter()
            .filter(|(_, pin)| pin.node.status == Status::Success)
            .collect();
        completed.sort_by(|(left_name, left), (right_name, right)| {
            right
                .completed_at
                .cmp(&left.completed_at)
                .then_with(|| left_name.cmp(right_name))
        });
        for (name, pin) in completed {
            let lines = pin_line_count(pin);
            if lines <= remaining {
                visible.push(name.clone());
                remaining -= lines;
            }
        }

        visible.sort();
        visible.dedup();
        visible
    }

    fn title_name(&self, visible: usize) -> String {
        let phase = self.phase.unwrap_or(Phase::LoadingConfiguration);
        let mut title = format!("Pin Progress · {}", phase.label());
        let total = self.pins.len();
        if total > 1 {
            if visible < total {
                title.push_str(&format!(" · showing {visible} of {total} pins"));
            } else {
                title.push_str(&format!(" · {total} pins"));
            }
        }
        title
    }

    fn pin_tree(&self, name: &str) -> Option<TreeNode> {
        let pin = self.pins.get(name)?;
        let mut children = Vec::new();
        if pin.sources_collapsed {
            for (package, node) in &pin.packages {
                children.push(TreeNode {
                    key: NodeKey::PinPackage(name.into(), package.clone()),
                    base: package.clone(),
                    node: node.clone(),
                    children: Vec::new(),
                });
            }
        } else {
            for (source_name, source) in &pin.sources {
                let mut packages = Vec::new();
                for (package, node) in &source.packages {
                    packages.push(TreeNode {
                        key: NodeKey::SourcePackage(name.into(), source_name.clone(), package.clone()),
                        base: package.clone(),
                        node: node.clone(),
                        children: Vec::new(),
                    });
                }
                children.push(TreeNode {
                    key: NodeKey::Source(name.into(), source_name.clone()),
                    base: source_name.clone(),
                    node: source.node.clone(),
                    children: packages,
                });
            }
        }
        Some(TreeNode {
            key: NodeKey::Pin(name.into()),
            base: pin_base(name, pin),
            node: pin.node.clone(),
            children,
        })
    }

    fn node_for_key(&self, key: &NodeKey) -> Option<(String, Node, usize)> {
        match key {
            NodeKey::Pin(name) => {
                let pin = self.pins.get(name)?;
                Some((pin_base(name, pin), pin.node.clone(), 1))
            }
            NodeKey::Source(pin, source) => {
                let source = self.pins.get(pin)?.sources.get(source)?;
                Some((source_name(key).to_owned(), source.node.clone(), 2))
            }
            NodeKey::PinPackage(pin, package) => {
                let node = self.pins.get(pin)?.packages.get(package)?;
                Some((package.clone(), node.clone(), 2))
            }
            NodeKey::SourcePackage(pin, source, package) => {
                let node = self.pins.get(pin)?.sources.get(source)?.packages.get(package)?;
                Some((package.clone(), node.clone(), 3))
            }
        }
    }

    fn write_snapshot(&mut self) {
        if !self.output_is_terminal {
            if let Some(phase) = self.phase.filter(|phase| *phase != Phase::Done) {
                let _ = writeln!(self.output, "{} failed", phase.label());
            }
            return;
        }
        self.refresh_dimensions();
        let trees = self.visible_trees();
        let title = self.title_name(trees.len());
        let failed_global = self.phase != Some(Phase::Done);
        let _ = if failed_global {
            writeln!(self.output, "⚠ {title}")
        } else {
            writeln!(self.output, "{title}")
        };
        render_snapshot_children(&mut self.output, &trees, "", self.terminal_dimensions.0);
    }

    fn log_pin(&mut self, name: &str) {
        if self.output_is_terminal {
            return;
        }
        if let Some(pin) = self.pins.get(name) {
            let line = format_node_text(&pin_base(name, pin), &pin.node, None);
            let _ = writeln!(self.output, "{line}");
        }
    }
}

pub struct Progress {
    state: Arc<Mutex<State>>,
    output_is_terminal: bool,
}

impl Progress {
    pub fn stderr() -> Self {
        let options = line::Options {
            frames_per_second: 10.0,
            throughput: true,
            initial_delay: None,
            hide_cursor: false,
            ..line::Options::default().auto_configure(StreamKind::Stderr)
        };
        Self::new(Box::new(io::stderr()), options, true, true)
    }

    fn new(output: Box<dyn Write + Send>, options: line::Options, auto_dimensions: bool, render: bool) -> Self {
        let root: Arc<Root> = root::Options {
            message_buffer_capacity: 1024,
            ..Default::default()
        }
        .into();
        let output_is_terminal = options.output_is_terminal;
        let terminal_dimensions = options.terminal_dimensions;
        let output = SharedWriter::new(output);
        let renderer_options = (output_is_terminal && render).then(|| options.clone());
        let renderer = if renderer_options.is_some() {
            catch_unwind(AssertUnwindSafe(|| {
                line::render(output.clone(), Arc::downgrade(&root), options)
            }))
            .ok()
        } else {
            None
        };
        Self {
            state: Arc::new(Mutex::new(State::new(
                root,
                output,
                output_is_terminal,
                terminal_dimensions,
                auto_dimensions,
                renderer_options,
                renderer,
            ))),
            output_is_terminal,
        }
    }

    pub fn reporter(&self) -> Reporter {
        Reporter {
            state: Arc::clone(&self.state),
        }
    }

    pub fn phase(&self, phase: Phase) {
        if let Ok(mut state) = self.state.lock() {
            state.set_phase(phase);
        }
    }

    pub fn complete(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.complete();
        }
    }

    pub fn finish(self) -> bool {
        self.shutdown();
        if let Ok(mut state) = self.state.lock() {
            state.write_snapshot();
        }
        self.output_is_terminal
    }

    fn shutdown(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.shutdown_renderer();
        }
    }

    #[cfg(test)]
    fn test_terminal(dimensions: (u16, u16)) -> (Self, Arc<Mutex<Vec<u8>>>) {
        Self::test(false, dimensions)
    }

    #[cfg(test)]
    fn test_non_terminal(dimensions: (u16, u16)) -> (Self, Arc<Mutex<Vec<u8>>>) {
        Self::test(true, dimensions)
    }

    #[cfg(test)]
    fn test(non_terminal: bool, dimensions: (u16, u16)) -> (Self, Arc<Mutex<Vec<u8>>>) {
        let output = Arc::new(Mutex::new(Vec::new()));
        let writer = BufferWriter(Arc::clone(&output));
        let options = line::Options {
            output_is_terminal: !non_terminal,
            terminal_dimensions: dimensions,
            throughput: true,
            ..Default::default()
        };
        (Self::new(Box::new(writer), options, false, false), output)
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
            state.pins.insert(
                name.into(),
                Pin {
                    node: Node::new(),
                    current: current.map(str::to_owned),
                    target: None,
                    sources_collapsed: false,
                    sources: BTreeMap::new(),
                    packages_collapsed: false,
                    packages: BTreeMap::new(),
                    completed_at: None,
                },
            );
        });
    }

    pub fn target(&self, name: &str, target: &str) {
        self.with_pin(name, |pin| pin.target = Some(target.into()));
    }

    pub fn step(&self, name: &str, step: PinStep) {
        self.with_pin(name, |pin| pin.node.set_step(step.label()));
    }

    pub fn detail(&self, name: &str, detail: Option<NixProgress>) {
        self.with_state_update(|state| {
            if let Some(pin) = state.pins.get_mut(name) {
                pin.node.set_detail(detail);
            }
        });
    }

    pub fn pause(&self, name: &str) {
        self.with_pin(name, |pin| pin.node.pause());
    }

    pub fn sources(&self, name: &str, sources: Vec<String>) {
        self.with_pin(name, |pin| {
            pin.sources.clear();
            pin.sources_collapsed = is_default_only(&sources);
            if !pin.sources_collapsed {
                for source in sources {
                    pin.sources.insert(
                        source,
                        Source {
                            node: Node::new(),
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
        self.with_state_update(|state| {
            let Some(pin) = state.pins.get_mut(name) else {
                return;
            };
            if pin.sources_collapsed && source == "default" {
                pin.node.set_detail(detail);
            } else if let Some(source) = pin.sources.get_mut(source) {
                source.node.set_detail(detail);
            }
        });
    }

    pub fn source_done(&self, name: &str, source: &str) {
        self.with_pin(name, |pin| {
            if pin.sources_collapsed && source == "default" {
                pin.node.clear_activity();
            } else if let Some(source) = pin.sources.get_mut(source) {
                source.node.succeed();
            }
        });
    }

    pub fn source_failed(&self, name: &str, source: &str) {
        self.with_pin(name, |pin| {
            if pin.sources_collapsed && source == "default" {
                pin.node.fail(None);
            } else if let Some(source) = pin.sources.get_mut(source) {
                source.node.fail(None);
            }
        });
    }

    pub fn source_packages(&self, name: &str, source: &str, packages: Vec<String>) {
        self.with_pin(name, |pin| {
            if pin.sources_collapsed {
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
        self.with_source_packages_update(name, source, |parent, packages, collapsed| {
            package_detail(parent, packages, collapsed, package, detail);
        });
    }

    pub fn source_package_ready(&self, name: &str, source: &str, package: &str, _key: &str) {
        self.with_source_packages(name, source, |parent, packages, collapsed| {
            finish_package(parent, packages, collapsed, package);
        });
    }

    pub fn source_package_reused(&self, name: &str, source: &str, package: &str, _key: &str) {
        self.source_package_ready(name, source, package, "");
    }

    pub fn source_package_failed(&self, name: &str, source: &str, package: &str, key: &str) {
        self.with_source_packages(name, source, |parent, packages, collapsed| {
            fail_package(parent, packages, collapsed, package, key);
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
        self.with_state_update(|state| {
            let Some(pin) = state.pins.get_mut(name) else {
                return;
            };
            package_detail(
                &mut pin.node,
                &mut pin.packages,
                pin.packages_collapsed,
                package,
                detail,
            );
        });
    }

    pub fn package_ready(&self, name: &str, package: &str, _key: &str) {
        self.with_pin(name, |pin| {
            finish_package(&mut pin.node, &mut pin.packages, pin.packages_collapsed, package);
        });
    }

    pub fn package_reused(&self, name: &str, package: &str, _key: &str) {
        self.package_ready(name, package, "");
    }

    pub fn package_failed(&self, name: &str, package: &str, key: &str) {
        self.with_pin(name, |pin| {
            fail_package(&mut pin.node, &mut pin.packages, pin.packages_collapsed, package, key);
        });
    }

    pub fn done(&self, name: &str, _packages: Option<usize>) {
        self.with_state(|state| {
            let Some(pin) = state.pins.get_mut(name) else {
                return;
            };
            if matches!(pin.node.status, Status::Success | Status::Failure) {
                return;
            }
            pin.node.succeed();
            state.completion_order = state.completion_order.saturating_add(1);
            pin.completed_at = Some(state.completion_order);
            state.log_pin(name);
        });
    }

    pub fn failed(&self, name: &str, location: impl Into<String>) {
        let location = location.into();
        self.with_state(|state| {
            let Some(pin) = state.pins.get_mut(name) else {
                return;
            };
            pin.node.fail(Some(location));
            if pin.completed_at.is_none() {
                state.completion_order = state.completion_order.saturating_add(1);
                pin.completed_at = Some(state.completion_order);
                state.log_pin(name);
            }
        });
    }

    fn with_source_packages(
        &self,
        name: &str,
        source: &str,
        update: impl FnOnce(&mut Node, &mut BTreeMap<String, Node>, bool),
    ) {
        self.with_pin(name, |pin| {
            if pin.sources_collapsed {
                update(&mut pin.node, &mut pin.packages, pin.packages_collapsed);
            } else if let Some(source) = pin.sources.get_mut(source) {
                update(&mut source.node, &mut source.packages, source.packages_collapsed);
            }
        });
    }

    fn with_source_packages_update(
        &self,
        name: &str,
        source: &str,
        update: impl FnOnce(&mut Node, &mut BTreeMap<String, Node>, bool),
    ) {
        self.with_state_update(|state| {
            let Some(pin) = state.pins.get_mut(name) else {
                return;
            };
            if pin.sources_collapsed {
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
            state.rebuild();
        }
    }

    fn with_state_update(&self, update: impl FnOnce(&mut State)) {
        if let Ok(mut state) = self.state.lock() {
            update(&mut state);
            state.refresh_items();
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
            packages.insert(name, Node::new());
        }
    } else {
        parent.clear_activity();
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

fn finish_package(parent: &mut Node, packages: &mut BTreeMap<String, Node>, collapsed: bool, package: &str) {
    if collapsed && package == "default" {
        parent.clear_activity();
    } else {
        packages.remove(package);
    }
}

fn fail_package(parent: &mut Node, packages: &mut BTreeMap<String, Node>, collapsed: bool, package: &str, key: &str) {
    if collapsed && package == "default" {
        parent.fail(Some(key.into()));
    } else if let Some(package) = packages.get_mut(package) {
        package.fail(Some(key.into()));
    }
}

fn pin_base(name: &str, pin: &Pin) -> String {
    let version = match (&pin.current, &pin.target) {
        (Some(current), Some(target)) if current != target => format!("{current} → {target}"),
        (Some(current), _) => current.clone(),
        (None, Some(target)) => format!("— → {target}"),
        (None, None) => "—".into(),
    };
    format!("{name} {version}")
}

fn source_name(key: &NodeKey) -> &str {
    match key {
        NodeKey::Source(_, source) => source,
        _ => "",
    }
}

fn pin_line_count(pin: &Pin) -> usize {
    let mut lines = 1;
    if pin.sources_collapsed {
        lines += pin.packages.len();
    } else {
        lines += pin.sources.len();
        lines += pin.sources.values().map(|source| source.packages.len()).sum::<usize>();
    }
    lines
}

fn materialize_tree(parent: &mut Item, tree: &TreeNode, level: usize, width: u16, items: &mut BTreeMap<NodeKey, Item>) {
    let mut item = parent.add_child("");
    apply_node(&mut item, &tree.base, &tree.node, level, width);
    for child in &tree.children {
        materialize_tree(&mut item, child, level + 1, width, items);
    }
    items.insert(tree.key.clone(), item);
}

fn apply_node(item: &mut Item, base: &str, node: &Node, level: usize, width: u16) {
    let counter = matches!(node.detail, Some(NixProgress::Counter { .. }));
    let available = usize::from(width).saturating_sub(level);
    let reserve = if counter {
        available.saturating_sub(4).min(24)
    } else {
        0
    };
    let name_width = available.saturating_sub(reserve).max(1);
    item.set_name(truncate_width(&format_node_text(base, node, None), name_width));
    match &node.detail {
        Some(NixProgress::Counter {
            current, total, unit, ..
        }) => {
            item.init(total.map(saturating_usize), Some(progress_unit(*unit)));
            item.set(saturating_usize(*current));
        }
        _ if node.status == Status::Active => item.init(None, None),
        _ => {}
    }
}

fn progress_unit(unit: NixProgressUnit) -> unit::Unit {
    match unit {
        NixProgressUnit::Bytes => {
            unit::dynamic_and_mode(prodash::unit::Bytes, Mode::with_percentage().and_throughput())
        }
        NixProgressUnit::Objects => unit::label_and_mode("objects", Mode::with_percentage().and_throughput()),
    }
}

fn format_node_text(base: &str, node: &Node, counter_values: Option<bool>) -> String {
    let mut text = format!("{} {base}", node.status.symbol());
    if let Some(step) = &node.step {
        text.push_str(" · ");
        text.push_str(step);
    }
    match &node.detail {
        Some(NixProgress::Text(detail)) => {
            text.push_str(" · ");
            text.push_str(detail);
        }
        Some(NixProgress::Counter {
            current,
            total,
            unit,
            label,
        }) => {
            text.push_str(" · ");
            text.push_str(label);
            if counter_values == Some(true) {
                text.push(' ');
                text.push_str(
                    &progress_unit(*unit)
                        .display(saturating_usize(*current), total.map(saturating_usize), None)
                        .to_string(),
                );
            }
        }
        None => {}
    }
    text
}

fn render_snapshot_children(output: &mut SharedWriter, children: &[TreeNode], prefix: &str, width: u16) {
    for (index, child) in children.iter().enumerate() {
        let last = index + 1 == children.len();
        let connector = if last { "└─ " } else { "├─ " };
        let line_prefix = format!("{prefix}{connector}");
        let available = usize::from(width).saturating_sub(UnicodeWidthStr::width(line_prefix.as_str()));
        let text = truncate_width(
            &format_node_text(&child.base, &child.node, Some(true)),
            available.max(1),
        );
        let _ = writeln!(output, "{line_prefix}{text}");
        let child_prefix = format!("{prefix}{}", if last { "   " } else { "│  " });
        render_snapshot_children(output, &child.children, &child_prefix, width);
    }
}

fn truncate_width(text: &str, max_width: usize) -> String {
    if UnicodeWidthStr::width(text) <= max_width {
        return text.into();
    }
    if max_width == 0 {
        return String::new();
    }
    if max_width == 1 {
        return "…".into();
    }
    let target = max_width - 1;
    let mut result = String::new();
    let mut width = 0;
    for character in text.chars() {
        let character_width = character.width().unwrap_or(0);
        if width + character_width > target {
            break;
        }
        result.push(character);
        width += character_width;
    }
    result.push('…');
    result
}

fn saturating_usize(value: u64) -> usize {
    usize::try_from(value).unwrap_or(usize::MAX)
}

#[cfg(test)]
#[derive(Clone)]
struct BufferWriter(Arc<Mutex<Vec<u8>>>);

#[cfg(test)]
impl Write for BufferWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().write(buffer)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn output(output: &Arc<Mutex<Vec<u8>>>) -> String {
        String::from_utf8(output.lock().unwrap().clone()).unwrap()
    }

    #[test]
    fn single_pin_finishes_as_static_tree() {
        let (progress, output_buffer) = Progress::test_terminal((80, 24));
        progress.phase(Phase::LoadingConfiguration);
        progress.phase(Phase::CheckingVersions);

        let reporter = progress.reporter();
        reporter.declare("demo", Some("v1"));
        reporter.step("demo", PinStep::Checking);
        reporter.target("demo", "v2");
        reporter.done("demo", None);
        progress.phase(Phase::WritingPinsFile);
        progress.complete();
        progress.finish();

        assert_eq!(output(&output_buffer), "Pin Progress · done\n└─ ✔ demo v1 → v2\n");
    }

    #[test]
    fn multi_source_failure_keeps_source_and_package_branch() {
        let (progress, output_buffer) = Progress::test_terminal((100, 30));
        progress.phase(Phase::CheckingVersions);
        let reporter = progress.reporter();
        reporter.declare("demo", Some("v1"));
        reporter.target("demo", "v2");
        reporter.sources("demo", vec!["api".into(), "web".into()]);
        reporter.source_done("demo", "api");
        reporter.source_packages("demo", "web", vec!["frontend".into(), "server".into()]);
        reporter.source_package_ready("demo", "web", "server", "vendorHash");
        reporter.source_package_failed("demo", "web", "frontend", "npmDepsHash");
        reporter.source_failed("demo", "web");
        reporter.failed("demo", "Source web/Package frontend/Derived Hash npmDepsHash");
        progress.complete();
        progress.finish();

        assert_eq!(
            output(&output_buffer),
            concat!(
                "Pin Progress · done\n",
                "└─ ⚠ demo v1 → v2 · Source web/Package frontend/Derived Hash npmDepsHash\n",
                "   ├─ ✔ api\n",
                "   └─ ⚠ web\n",
                "      └─ ⚠ frontend · npmDepsHash\n",
            )
        );
    }

    #[test]
    fn height_budget_keeps_active_and_recent_successful_pins() {
        let (progress, output_buffer) = Progress::test_terminal((80, 12));
        progress.phase(Phase::CheckingVersions);
        let reporter = progress.reporter();
        for name in ["alpha", "beta", "gamma", "omega"] {
            reporter.declare(name, Some("v1"));
        }
        reporter.done("alpha", None);
        reporter.done("beta", None);
        reporter.done("gamma", None);
        reporter.step("omega", PinStep::Checking);
        progress.complete();
        progress.finish();

        assert_eq!(
            output(&output_buffer),
            concat!(
                "Pin Progress · done · showing 3 of 4 pins\n",
                "├─ ✔ beta v1\n",
                "├─ ✔ gamma v1\n",
                "└─ ⏵ omega v1 · Checking\n",
            )
        );
    }

    #[test]
    fn non_terminal_output_is_linear_and_low_frequency() {
        let (progress, output_buffer) = Progress::test_non_terminal((80, 24));
        progress.phase(Phase::LoadingConfiguration);
        progress.phase(Phase::CheckingVersions);
        let reporter = progress.reporter();
        reporter.declare("demo", None);
        reporter.target("demo", "v1");
        reporter.done("demo", None);
        progress.phase(Phase::WritingPinsFile);
        progress.complete();
        progress.finish();

        assert_eq!(
            output(&output_buffer),
            concat!(
                "Loading configuration done\n",
                "✔ demo — → v1\n",
                "Checking versions done\n",
                "Writing pins.json done\n",
            )
        );
    }

    #[test]
    fn unicode_text_is_truncated_by_display_width() {
        assert_eq!(truncate_width("✔ 示例-example", 8), "✔ 示例-…");
    }

    #[test]
    fn pause_restores_waiting_status() {
        let (progress, output_buffer) = Progress::test_terminal((80, 24));
        progress.phase(Phase::CheckingVersions);
        let reporter = progress.reporter();
        reporter.declare("demo", Some("v1"));
        reporter.step("demo", PinStep::Checking);
        reporter.pause("demo");
        progress.complete();
        progress.finish();

        assert_eq!(output(&output_buffer), "Pin Progress · done\n└─ ⏸ demo v1\n");
    }

    #[test]
    fn collapsed_package_failure_logs_full_pin_location_once() {
        let (progress, output_buffer) = Progress::test_non_terminal((80, 24));
        progress.phase(Phase::ResolvingDerivedHashes);
        let reporter = progress.reporter();
        reporter.declare("demo", Some("v1"));
        reporter.sources("demo", vec!["default".into()]);
        reporter.packages("demo", vec!["default".into()]);
        reporter.package_failed("demo", "default", "vendorHash");
        reporter.failed("demo", "Source default/Package default/Derived Hash vendorHash");

        let text = output(&output_buffer);
        assert_eq!(text.matches("⚠ demo").count(), 1, "{text}");
        assert!(
            text.contains("Source default/Package default/Derived Hash vendorHash"),
            "{text}"
        );
    }

    #[test]
    fn unfinished_global_phase_is_rendered_as_failure_without_pins() {
        let (progress, output_buffer) = Progress::test_terminal((80, 24));
        progress.phase(Phase::LoadingConfiguration);
        progress.finish();

        assert_eq!(output(&output_buffer), "⚠ Pin Progress · Loading configuration\n");
    }

    #[test]
    fn zero_height_uses_twenty_row_fallback() {
        let (progress, _) = Progress::test_terminal((80, 0));
        progress.phase(Phase::CheckingVersions);
        let reporter = progress.reporter();
        for index in 0..20 {
            reporter.declare(&format!("pin-{index:02}"), None);
        }

        assert_eq!(progress.state.lock().unwrap().visible_pin_names().len(), 19);
    }

    #[test]
    fn multi_source_subtree_is_pruned_as_a_whole() {
        let (progress, _) = Progress::test_terminal((80, 9));
        progress.phase(Phase::CheckingVersions);
        let reporter = progress.reporter();
        reporter.declare("demo", None);
        reporter.sources("demo", vec!["api".into(), "web".into()]);

        assert!(progress.state.lock().unwrap().visible_pin_names().is_empty());
    }

    #[test]
    fn non_terminal_failure_logs_the_active_phase() {
        let (progress, output_buffer) = Progress::test_non_terminal((80, 24));
        progress.phase(Phase::LoadingConfiguration);
        progress.finish();

        assert_eq!(output(&output_buffer), "Loading configuration failed\n");
    }

    #[test]
    fn counter_update_keeps_the_same_visible_tree() {
        let (progress, _) = Progress::test_terminal((80, 24));
        progress.phase(Phase::CheckingVersions);
        let reporter = progress.reporter();
        reporter.declare("demo", Some("v1"));
        reporter.detail(
            "demo",
            Some(NixProgress::Counter {
                current: 13,
                total: Some(31),
                unit: NixProgressUnit::Objects,
                label: "Git objects".into(),
            }),
        );
        let before = {
            let state = progress.state.lock().unwrap();
            let mut snapshot = Vec::new();
            state.root.sorted_snapshot(&mut snapshot);
            snapshot.into_iter().map(|(key, _)| key).collect::<Vec<_>>()
        };

        reporter.detail(
            "demo",
            Some(NixProgress::Counter {
                current: 17,
                total: Some(31),
                unit: NixProgressUnit::Objects,
                label: "Git objects".into(),
            }),
        );

        let after = {
            let state = progress.state.lock().unwrap();
            let mut snapshot = Vec::new();
            state.root.sorted_snapshot(&mut snapshot);
            snapshot.into_iter().map(|(key, _)| key).collect::<Vec<_>>()
        };
        assert_eq!(before, after);
    }
}
