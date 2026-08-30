use crate::nix::{NixProgress, NixProgressUnit};
use crosstermion::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use futures_lite::Stream;
use prodash::messages::MessageLevel;
use prodash::render::tui;
use prodash::tree::{root, Item, Root};
use prodash::unit::{self, display::Mode};
use std::collections::BTreeMap;
use std::io::{self, IsTerminal, Write};
use std::pin::Pin as FuturePin;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::task::{Context, Poll};
use std::thread;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

static CANCELLED: AtomicBool = AtomicBool::new(false);

pub fn cancelled() -> bool {
    CANCELLED.load(Ordering::SeqCst)
}

pub(crate) fn command_output(command: &mut Command) -> io::Result<Output> {
    command_output_with_cancel(command, &CANCELLED)
}

fn command_output_with_cancel(command: &mut Command, cancelled: &AtomicBool) -> io::Result<Output> {
    use std::io::Read;
    use std::time::Duration;

    configure_child_process(command);
    let mut child = command.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("无法读取子进程 stdout"))?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| io::Error::other("无法读取子进程 stderr"))?;
    let stdout = thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout.read_to_end(&mut bytes).map(|_| bytes)
    });
    let stderr = thread::spawn(move || {
        let mut bytes = Vec::new();
        stderr.read_to_end(&mut bytes).map(|_| bytes)
    });
    let status = loop {
        if cancelled.load(Ordering::SeqCst) {
            kill_child_tree(&mut child);
            break child.wait()?;
        }
        if let Some(status) = child.try_wait()? {
            break status;
        }
        thread::sleep(Duration::from_millis(25));
    };
    let stdout = stdout
        .join()
        .map_err(|_| io::Error::other("读取子进程 stdout 的线程异常退出"))??;
    let stderr = stderr
        .join()
        .map_err(|_| io::Error::other("读取子进程 stderr 的线程异常退出"))??;
    Ok(Output { status, stdout, stderr })
}

pub(crate) fn configure_child_process(command: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
}

pub(crate) fn kill_child_tree(child: &mut std::process::Child) {
    #[cfg(unix)]
    {
        // SAFETY: 子进程在 spawn 前已进入以自身 pid 为 id 的独立进程组。
        if unsafe { libc::kill(-(child.id() as i32), libc::SIGKILL) } == -1 {
            let _ = child.kill();
        }
    }
    #[cfg(not(unix))]
    let _ = child.kill();
}

fn cancel() {
    CANCELLED.store(true, Ordering::SeqCst);
}

fn tui_dimensions_supported((width, height): (u16, u16)) -> bool {
    width >= 60 && height >= 12
}

struct EventStream {
    receiver: mpsc::Receiver<tui::Event>,
}

impl Stream for EventStream {
    type Item = tui::Event;

    fn poll_next(self: FuturePin<&mut Self>, _: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match self.receiver.try_recv() {
            Ok(event) => Poll::Ready(Some(event)),
            Err(mpsc::TryRecvError::Empty) => Poll::Pending,
            Err(mpsc::TryRecvError::Disconnected) => Poll::Ready(None),
        }
    }
}

struct TuiRenderer {
    events: mpsc::Sender<tui::Event>,
    shutting_down: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl TuiRenderer {
    fn start(root: &Arc<Root>) -> io::Result<Self> {
        let (events, receiver) = mpsc::channel();
        let options = tui::Options {
            title: "Nix Pins".into(),
            throughput: true,
            recompute_column_width_every_nth_frame: Some(10),
            ..Default::default()
        };
        let render = tui::render_with_input(io::stderr(), Arc::downgrade(root), options, EventStream { receiver })?;
        let shutting_down = Arc::new(AtomicBool::new(false));
        let renderer_stopping = Arc::clone(&shutting_down);
        let handle = thread::spawn(move || {
            futures_lite::future::block_on(render);
            if !renderer_stopping.load(Ordering::SeqCst) {
                cancel();
            }
        });
        Ok(Self {
            events,
            shutting_down,
            handle: Some(handle),
        })
    }

    fn event_sender(&self) -> mpsc::Sender<tui::Event> {
        self.events.clone()
    }

    fn shutdown(&mut self) {
        self.shutting_down.store(true, Ordering::SeqCst);
        let _ = self
            .events
            .send(tui::Event::Input(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE)));
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

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
            Self::Active => "",
            Self::Success => "✔",
            Self::Failure => "⚠",
        }
    }
}

#[derive(Clone)]
struct Node {
    status: Status,
    step: Option<String>,
    detail: Option<NixProgress>,
    activities: Vec<String>,
}

impl Node {
    fn new() -> Self {
        Self {
            status: Status::Waiting,
            step: None,
            detail: None,
            activities: Vec::new(),
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
        match detail {
            Some(NixProgress::Status { detail, activities }) => {
                self.detail = detail.map(|detail| *detail);
                self.activities = activities;
            }
            detail => {
                self.detail = detail;
                self.activities.clear();
            }
        }
    }

    fn clear_activity(&mut self) {
        self.step = None;
        self.detail = None;
        self.activities.clear();
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
        self.activities.clear();
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

impl TreeNode {
    fn new(key: NodeKey, base: String, node: Node, children: Vec<TreeNode>) -> Self {
        Self {
            key,
            base,
            node,
            children,
        }
    }
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
    events: Option<mpsc::Sender<tui::Event>>,
    phase: Option<Phase>,
    pins: BTreeMap<String, Pin>,
    items: BTreeMap<NodeKey, Item>,
    activity_state: BTreeMap<NodeKey, Vec<String>>,
    completion_order: u64,
}

struct SpinnerUnit;

impl unit::DisplayValue for SpinnerUnit {
    fn display_current_value(&self, _: &mut dyn std::fmt::Write, _: usize, _: Option<usize>) -> std::fmt::Result {
        Ok(())
    }

    fn dyn_hash(&self, _: &mut dyn std::hash::Hasher) {}

    fn display_unit(&self, _: &mut dyn std::fmt::Write, _: usize) -> std::fmt::Result {
        Ok(())
    }
}

impl State {
    fn new(
        root: Arc<Root>,
        output: SharedWriter,
        output_is_terminal: bool,
        terminal_dimensions: (u16, u16),
        events: Option<mpsc::Sender<tui::Event>>,
    ) -> Self {
        Self {
            root,
            output,
            output_is_terminal,
            terminal_dimensions,
            events,
            phase: None,
            pins: BTreeMap::new(),
            items: BTreeMap::new(),
            activity_state: BTreeMap::new(),
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

    fn rebuild(&mut self) {
        if !self.output_is_terminal {
            return;
        }
        self.items.clear();

        let trees = self.visible_trees();
        let mut items = BTreeMap::new();
        for tree in &trees {
            materialize_root(&self.root, tree, self.terminal_dimensions.0, &mut items);
        }
        self.items = items;
        self.sync_activity_feed();
        self.send_information();
    }

    fn refresh_items(&mut self) {
        if !self.output_is_terminal {
            return;
        }
        let mut expected_keys = Vec::new();
        for tree in self.visible_trees() {
            collect_tree_keys(&tree, &mut expected_keys);
        }
        expected_keys.sort();
        if expected_keys != self.items.keys().cloned().collect::<Vec<_>>() {
            self.rebuild();
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
        self.sync_activity_feed();
        self.send_information();
    }

    fn sync_activity_feed(&mut self) {
        let mut current = BTreeMap::new();
        for tree in self.visible_trees() {
            collect_activities(&tree, &mut current);
        }
        for (key, activities) in &current {
            let previous = self.activity_state.get(key);
            for activity in activities {
                if !previous.is_some_and(|previous| previous.contains(activity)) {
                    if let Some(item) = self.items.get(key) {
                        item.message(activity_message_level(activity), sanitize_activity(activity));
                    }
                }
            }
        }
        self.activity_state = current;
    }

    fn information_lines(&self) -> Vec<tui::Line> {
        let completed = self
            .pins
            .values()
            .filter(|pin| pin.node.status == Status::Success)
            .count();
        let failed = self
            .pins
            .values()
            .filter(|pin| pin.node.status == Status::Failure)
            .count();
        let activities = self.activity_state.values().flatten();
        let builds = activities.clone().filter(|line| line.starts_with("Building")).count();
        let copies = activities.clone().filter(|line| line.starts_with("Copying")).count();
        let queries = activities.filter(|line| line.starts_with("Querying")).count();
        let downloads = self.pins.values().map(count_pin_downloads).sum::<usize>();
        vec![
            tui::Line::Title("Update".into()),
            tui::Line::Text(format!(
                "Phase: {}",
                self.phase.unwrap_or(Phase::LoadingConfiguration).label()
            )),
            tui::Line::Text(format!("Pins: {completed}/{} · {failed} failed", self.pins.len())),
            tui::Line::Title("Nix".into()),
            tui::Line::Text(format!(
                "Build {builds} · Download {downloads} · Copy {copies} · Query {queries}"
            )),
            tui::Line::Title("Keys".into()),
            tui::Line::Text("j/k scroll · q/Esc/Ctrl+C cancel".into()),
        ]
    }

    fn send_information(&self) {
        if let Some(events) = &self.events {
            let _ = events.send(tui::Event::SetInformation(self.information_lines()));
        }
    }

    fn visible_trees(&self) -> Vec<TreeNode> {
        self.visible_pin_names()
            .into_iter()
            .filter_map(|name| self.pin_tree(&name))
            .collect()
    }

    fn visible_pin_names(&self) -> Vec<String> {
        self.pins.keys().cloned().collect()
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
                children.push(TreeNode::new(
                    NodeKey::PinPackage(name.into(), package.clone()),
                    package.clone(),
                    node.clone(),
                    Vec::new(),
                ));
            }
        } else {
            for (source_name, source) in &pin.sources {
                let mut packages = Vec::new();
                for (package, node) in &source.packages {
                    packages.push(TreeNode::new(
                        NodeKey::SourcePackage(name.into(), source_name.clone(), package.clone()),
                        package.clone(),
                        node.clone(),
                        Vec::new(),
                    ));
                }
                children.push(TreeNode::new(
                    NodeKey::Source(name.into(), source_name.clone()),
                    source_name.clone(),
                    source.node.clone(),
                    packages,
                ));
            }
        }
        Some(TreeNode::new(
            NodeKey::Pin(name.into()),
            pin_base(name, pin),
            pin.node.clone(),
            children,
        ))
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

    fn complete_pin(&mut self, name: &str) {
        let Some(pin) = self.pins.get_mut(name) else {
            return;
        };
        if matches!(pin.node.status, Status::Success | Status::Failure) {
            return;
        }
        pin.node.succeed();
        self.completion_order = self.completion_order.saturating_add(1);
        pin.completed_at = Some(self.completion_order);
        self.log_pin(name);
    }
}

pub struct Progress {
    state: Arc<Mutex<State>>,
    output_is_terminal: bool,
    renderer: Mutex<Option<TuiRenderer>>,
}

impl Progress {
    pub fn stderr() -> Result<Self, ctrlc::Error> {
        CANCELLED.store(false, Ordering::SeqCst);
        let dimensions = crosstermion::crossterm::terminal::size().unwrap_or((80, 24));
        let render = io::stderr().is_terminal() && tui_dimensions_supported(dimensions);
        let progress = Self::new(Box::new(io::stderr()), dimensions, render, render);
        let events = progress
            .renderer
            .lock()
            .ok()
            .and_then(|renderer| renderer.as_ref().map(TuiRenderer::event_sender));
        ctrlc::set_handler(move || {
            cancel();
            if let Some(events) = &events {
                let _ = events.send(tui::Event::Input(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE)));
            }
        })?;
        Ok(progress)
    }

    fn new(
        output: Box<dyn Write + Send>,
        terminal_dimensions: (u16, u16),
        render: bool,
        terminal_output: bool,
    ) -> Self {
        let root: Arc<Root> = root::Options {
            message_buffer_capacity: 1024,
            ..Default::default()
        }
        .into();
        let renderer = render.then(|| TuiRenderer::start(&root)).transpose().ok().flatten();
        let output_is_terminal = if render { renderer.is_some() } else { terminal_output };
        let events = renderer.as_ref().map(TuiRenderer::event_sender);
        let state = Arc::new(Mutex::new(State::new(
            root,
            SharedWriter::new(output),
            output_is_terminal,
            terminal_dimensions,
            events,
        )));
        Self {
            state,
            output_is_terminal,
            renderer: Mutex::new(renderer),
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
        if let Ok(mut renderer) = self.renderer.lock() {
            if let Some(mut renderer) = renderer.take() {
                renderer.shutdown();
            }
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
        (Self::new(Box::new(writer), dimensions, false, !non_terminal), output)
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
        self.with_state(|state| {
            let Some(pin) = state.pins.get_mut(name) else {
                return;
            };
            let completed = if pin.sources_collapsed && source == "default" {
                pin.node.clear_activity();
                true
            } else {
                let Some(source) = pin.sources.get_mut(source) else {
                    return;
                };
                source.node.succeed();
                pin.sources.values().all(|source| source.node.status == Status::Success)
            };
            if completed {
                state.complete_pin(name);
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
        self.with_state(|state| state.complete_pin(name));
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

fn collect_tree_keys(tree: &TreeNode, keys: &mut Vec<NodeKey>) {
    keys.push(tree.key.clone());
    for child in &tree.children {
        collect_tree_keys(child, keys);
    }
}

fn collect_activities(tree: &TreeNode, activities: &mut BTreeMap<NodeKey, Vec<String>>) {
    if !tree.node.activities.is_empty() {
        activities.insert(tree.key.clone(), tree.node.activities.clone());
    }
    for child in &tree.children {
        collect_activities(child, activities);
    }
}

fn count_node_download(node: &Node) -> usize {
    usize::from(matches!(
        node.detail,
        Some(NixProgress::Counter {
            unit: NixProgressUnit::Bytes | NixProgressUnit::Objects,
            ..
        })
    ))
}

fn activity_message_level(activity: &str) -> MessageLevel {
    let activity = activity.trim_start().to_ascii_lowercase();
    if ["error:", "fatal:", "warning:", "warn:"]
        .iter()
        .any(|prefix| activity.starts_with(prefix))
    {
        MessageLevel::Failure
    } else {
        MessageLevel::Info
    }
}

fn count_pin_downloads(pin: &Pin) -> usize {
    count_node_download(&pin.node)
        + pin
            .sources
            .values()
            .map(|source| {
                count_node_download(&source.node) + source.packages.values().map(count_node_download).sum::<usize>()
            })
            .sum::<usize>()
        + pin.packages.values().map(count_node_download).sum::<usize>()
}

pub(crate) fn sanitize_activity(activity: &str) -> String {
    static URL: OnceLock<regex::Regex> = OnceLock::new();
    URL.get_or_init(|| {
        regex::Regex::new(
            r"(?P<scheme>[A-Za-z][A-Za-z0-9+.-]*://)(?:[^/@\s]+@)?(?P<host>[^/?#\s]+)(?P<path>/[^?#\s]*)?(?:\?[^#\s]*)?(?:#[^\s]*)?",
        )
        .expect("固定 URL 正则必须有效")
    })
    .replace_all(activity, "$scheme$host$path")
    .into_owned()
}

fn materialize_root(root: &Root, tree: &TreeNode, width: u16, items: &mut BTreeMap<NodeKey, Item>) {
    let mut item = root.add_child("");
    apply_node(&mut item, &tree.base, &tree.node, 0, width);
    for child in &tree.children {
        materialize_tree(&mut item, child, 1, width, items);
    }
    items.insert(tree.key.clone(), item);
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
    let active = node.status == Status::Active && !counter;
    let available = usize::from(width).saturating_sub(level);
    let reserve = if counter || active {
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
        _ if active => item.init(None, Some(unit::dynamic(SpinnerUnit))),
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
    let symbol = node.status.symbol();
    let mut text = if symbol.is_empty() {
        base.to_owned()
    } else {
        format!("{symbol} {base}")
    };
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
        Some(NixProgress::Status { .. }) | None => {}
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
    fn small_terminal_snapshot_keeps_all_pins() {
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
                "Pin Progress · done · 4 pins\n",
                "├─ ✔ alpha v1\n",
                "├─ ✔ beta v1\n",
                "├─ ✔ gamma v1\n",
                "└─ omega v1 · Checking\n",
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
    fn last_source_finishes_pin_immediately() {
        let (progress, _) = Progress::test_terminal((80, 24));
        progress.phase(Phase::ResolvingDerivedHashes);
        let reporter = progress.reporter();
        reporter.declare("demo", Some("v1"));
        reporter.sources("demo", vec!["api".into(), "web".into()]);

        reporter.source_done("demo", "api");
        assert_eq!(progress.state.lock().unwrap().pins["demo"].node.status, Status::Waiting);

        reporter.source_done("demo", "web");
        assert_eq!(progress.state.lock().unwrap().pins["demo"].node.status, Status::Success);
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
    fn terminal_height_does_not_hide_pins() {
        let (progress, _) = Progress::test_terminal((80, 0));
        progress.phase(Phase::CheckingVersions);
        let reporter = progress.reporter();
        for index in 0..20 {
            reporter.declare(&format!("pin-{index:02}"), None);
        }

        assert_eq!(progress.state.lock().unwrap().visible_pin_names().len(), 20);
    }

    #[test]
    fn narrow_terminal_falls_back_to_plain() {
        assert!(!tui_dimensions_supported((59, 20)));
        assert!(!tui_dimensions_supported((80, 11)));
        assert!(tui_dimensions_supported((60, 12)));
    }

    #[test]
    fn cancellation_stops_a_running_child() {
        let cancelled = AtomicBool::new(true);
        let mut command = Command::new("sh");
        command.args(["-c", "sleep 5"]);

        let started = std::time::Instant::now();
        let output = command_output_with_cancel(&mut command, &cancelled).unwrap();

        assert!(!output.status.success());
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
    }

    #[test]
    fn multi_source_subtree_remains_visible() {
        let (progress, _) = Progress::test_terminal((80, 9));
        progress.phase(Phase::CheckingVersions);
        let reporter = progress.reporter();
        reporter.declare("demo", None);
        reporter.sources("demo", vec!["api".into(), "web".into()]);

        assert_eq!(progress.state.lock().unwrap().visible_pin_names(), vec!["demo"]);
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

    #[test]
    fn activities_use_feed_without_duplicate_tree_rows() {
        let (progress, _) = Progress::test_terminal((80, 24));
        progress.phase(Phase::CheckingVersions);
        let reporter = progress.reporter();
        reporter.declare("demo", Some("v1"));
        reporter.detail(
            "demo",
            Some(NixProgress::Status {
                detail: Some(Box::new(NixProgress::Text("Building".into()))),
                activities: vec!["Building https://user:secret@example.com/src.tar?token=hidden · buildPhase".into()],
            }),
        );

        let state = progress.state.lock().unwrap();
        let pin_key = NodeKey::Pin("demo".into());
        assert!(state.items[&pin_key].unit().is_some());
        assert_eq!(state.items.len(), 1);
        let mut messages = Vec::new();
        state.root.copy_messages(&mut messages);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].message, "Building https://example.com/src.tar · buildPhase");
    }

    #[test]
    fn activity_urls_keep_only_scheme_host_and_path() {
        assert_eq!(
            sanitize_activity("ssh://alice:secret@example.com/repo.git?token=hidden https://bob@example.net/a#x"),
            "ssh://example.com/repo.git https://example.net/a"
        );
    }

    #[test]
    fn diagnostics_keep_failure_severity_in_the_feed() {
        let (progress, _) = Progress::test_terminal((80, 24));
        progress.phase(Phase::CheckingVersions);
        let reporter = progress.reporter();
        reporter.declare("demo", None);
        reporter.detail(
            "demo",
            Some(NixProgress::Status {
                detail: None,
                activities: vec!["warning: cache is stale".into()],
            }),
        );

        let state = progress.state.lock().unwrap();
        let mut messages = Vec::new();
        state.root.copy_messages(&mut messages);
        assert_eq!(messages[0].level, MessageLevel::Failure);
    }

    #[test]
    fn information_summarizes_progress_and_active_nix_work() {
        let (progress, _) = Progress::test_terminal((100, 30));
        progress.phase(Phase::ResolvingSources);
        let reporter = progress.reporter();
        reporter.declare("done", None);
        reporter.done("done", None);
        reporter.declare("active", None);
        reporter.detail(
            "active",
            Some(NixProgress::Status {
                detail: Some(Box::new(NixProgress::Counter {
                    current: 10,
                    total: Some(20),
                    unit: NixProgressUnit::Bytes,
                    label: "Downloading".into(),
                })),
                activities: vec!["Building demo.drv".into(), "Querying Cache".into()],
            }),
        );
        reporter.declare("git", None);
        reporter.detail(
            "git",
            Some(NixProgress::Counter {
                current: 2,
                total: Some(4),
                unit: NixProgressUnit::Objects,
                label: "Git objects".into(),
            }),
        );

        assert_eq!(
            progress.state.lock().unwrap().information_lines(),
            vec![
                tui::Line::Title("Update".into()),
                tui::Line::Text("Phase: Resolving sources".into()),
                tui::Line::Text("Pins: 1/3 · 0 failed".into()),
                tui::Line::Title("Nix".into()),
                tui::Line::Text("Build 1 · Download 2 · Copy 0 · Query 1".into()),
                tui::Line::Title("Keys".into()),
                tui::Line::Text("j/k scroll · q/Esc/Ctrl+C cancel".into()),
            ]
        );
    }
}
