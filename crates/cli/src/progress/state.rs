//! Pin/Source/Package 状态树、消息投影与 Reporter 状态更新。

use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PinStep {
    Checking,
    ResolvingSources,
    ResolvingDerivedHashes,
    VersionSelected,
    HashingSource,
    PatchingSource,
    SourceReady,
    SourceReused,
    DerivedReused(String),
    HashingDerived(String),
}

impl PinStep {
    pub(super) fn label(&self) -> String {
        match self {
            Self::Checking => "Checking".into(),
            Self::ResolvingSources => "Resolving sources".into(),
            Self::ResolvingDerivedHashes => "Resolving derived hashes".into(),
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
    ProcessingPins,
    WritingPinsFile,
    Done,
}

impl Phase {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::LoadingConfiguration => "Loading configuration",
            Self::CheckingVersions => "Checking versions",
            Self::ProcessingPins => "Processing pins",
            Self::WritingPinsFile => "Writing pins.json",
            Self::Done => "done",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Status {
    Waiting,
    Active,
    Success,
    Failure,
}

impl Status {
    pub(super) fn symbol(self) -> &'static str {
        match self {
            Self::Waiting => "○",
            Self::Active => "",
            Self::Success => "✔",
            Self::Failure => "⚠",
        }
    }
}

/// 显示节点保留阶段、计数和诊断消息；等待或终态会清除上一阶段的活动信息。
#[derive(Clone)]
pub(super) struct Node {
    pub(super) status: Status,
    pub(super) step: Option<String>,
    pub(super) detail: Option<NixProgress>,
    pub(super) activities: Vec<String>,
    pub(super) message: Option<(MessageLevel, String)>,
}

impl Node {
    pub(super) fn new(waiting: &str) -> Self {
        Self {
            status: Status::Waiting,
            step: Some(waiting.into()),
            detail: None,
            activities: Vec::new(),
            message: None,
        }
    }

    pub(super) fn set_step(&mut self, step: impl Into<String>) {
        self.status = Status::Active;
        self.step = Some(step.into());
    }

    /// 诊断消息不改变任务状态；进度快照替换活动列表，避免残留已经结束的 Nix 活动。
    pub(super) fn set_detail(&mut self, detail: Option<NixProgress>) {
        if let Some(NixProgress::Message { level, text }) = detail {
            self.message = Some((level, text));
            return;
        }
        if detail.is_some() {
            if self.status == Status::Waiting {
                self.step = None;
            }
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

    pub(super) fn clear_activity(&mut self) {
        self.step = None;
        self.detail = None;
        self.activities.clear();
    }

    pub(super) fn wait(&mut self, reason: &str) {
        self.status = Status::Waiting;
        self.clear_activity();
        self.step = Some(reason.into());
    }

    pub(super) fn succeed(&mut self) {
        self.status = Status::Success;
        self.clear_activity();
    }

    pub(super) fn fail(&mut self, step: Option<String>) {
        self.status = Status::Failure;
        self.step = step;
        self.detail = None;
        self.activities.clear();
    }
}

pub(super) struct Source {
    pub(super) node: Node,
    pub(super) packages_collapsed: bool,
    pub(super) packages: BTreeMap<String, Node>,
}

pub(super) struct Pin {
    pub(super) node: Node,
    pub(super) current: Option<String>,
    pub(super) target: Option<String>,
    pub(super) revision: bool,
    pub(super) sources_collapsed: bool,
    pub(super) sources: BTreeMap<String, Source>,
    pub(super) packages_collapsed: bool,
    pub(super) packages: BTreeMap<String, Node>,
    pub(super) completed_at: Option<u64>,
}

/// 以 Pin/Source/Package 的完整归属标识节点，避免同名 Package 在不同 Source 间串状态。
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum NodeKey {
    Pin(String),
    Source(String, String),
    PinPackage(String, String),
    SourcePackage(String, String, String),
}

#[derive(Clone)]
pub(super) struct TreeNode {
    pub(super) key: NodeKey,
    pub(super) base: String,
    pub(super) node: Node,
    pub(super) children: Vec<TreeNode>,
}

impl TreeNode {
    pub(super) fn new(key: NodeKey, base: String, node: Node, children: Vec<TreeNode>) -> Self {
        Self {
            key,
            base,
            node,
            children,
        }
    }
}

/// 应用状态是显示事实来源，Prodash 节点和 Ratatui 快照都由它派生。
pub(super) struct State {
    pub(super) root: Arc<Root>,
    pub(super) output: SharedWriter,
    pub(super) output_is_terminal: bool,
    pub(super) terminal_dimensions: (u16, u16),
    pub(super) phase: Option<Phase>,
    pub(super) pins: BTreeMap<String, Pin>,
    pub(super) items: BTreeMap<NodeKey, Item>,
    pub(super) activity_state: BTreeMap<NodeKey, Vec<String>>,
    pub(super) completion_order: u64,
}

impl State {
    pub(super) fn resize(&mut self, dimensions: (u16, u16)) {
        if self.terminal_dimensions != dimensions {
            self.terminal_dimensions = dimensions;
        }
    }

    /// 渲染失败后切换到 plain 输出并补报已完成的 Pin，保留其余任务继续处理。
    pub(super) fn fallback(&mut self) {
        if !self.output_is_terminal {
            return;
        }
        self.items.clear();
        self.output_is_terminal = false;
        for name in self.visible_pin_names() {
            if self.pins[&name].completed_at.is_some() {
                self.log_pin(&name);
            }
        }
    }

    pub(super) fn new(
        root: Arc<Root>,
        output: SharedWriter,
        output_is_terminal: bool,
        terminal_dimensions: (u16, u16),
    ) -> Self {
        Self {
            root,
            output,
            output_is_terminal,
            terminal_dimensions,
            phase: None,
            pins: BTreeMap::new(),
            items: BTreeMap::new(),
            activity_state: BTreeMap::new(),
            completion_order: 0,
        }
    }

    pub(super) fn set_phase(&mut self, phase: Phase) {
        if self.phase == Some(phase) {
            return;
        }
        if !self.output_is_terminal
            && let Some(previous) = self.phase.filter(|phase| *phase != Phase::Done)
        {
            self.log_phase_done(previous);
        }
        self.phase = Some(phase);
        self.rebuild();
    }

    pub(super) fn complete(&mut self) {
        if self.phase == Some(Phase::Done) {
            return;
        }
        if !self.output_is_terminal
            && let Some(previous) = self.phase
        {
            self.log_phase_done(previous);
        }
        self.phase = Some(Phase::Done);
        self.rebuild();
    }

    pub(super) fn rebuild(&mut self) {
        if !self.output_is_terminal {
            return;
        }
        self.items.clear();

        let trees = self.visible_trees();
        let mut items = BTreeMap::new();
        for tree in &trees {
            materialize_root(&self.root, tree, &mut items);
        }
        self.items = items;
        self.sync_activity_feed();
    }

    /// 树结构不变时原位更新计数；只有节点集合变化才重建，避免刷新打断进度状态。
    pub(super) fn refresh_items(&mut self) {
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
        let keys: Vec<_> = self.items.keys().cloned().collect();
        for key in keys {
            let Some((base, node, _)) = self.node_for_key(&key) else {
                continue;
            };
            if let Some(item) = self.items.get_mut(&key) {
                apply_node(item, &base, &node);
            }
        }
        self.sync_activity_feed();
    }

    /// 活动以节点为单位比较上一份快照，持续存在的活动只在进入时写入消息缓冲区。
    pub(super) fn sync_activity_feed(&mut self) {
        let mut current = BTreeMap::new();
        for tree in self.visible_trees() {
            collect_activities(&tree, &mut current);
        }
        for (key, activities) in &current {
            let previous = self.activity_state.get(key);
            for activity in activities {
                if !previous.is_some_and(|previous| previous.contains(activity))
                    && let Some(item) = self.items.get(key)
                {
                    item.message(activity_message_level(activity), activity_summary(activity));
                }
            }
        }
        for (name, pin) in &mut self.pins {
            let mut nodes = vec![(NodeKey::Pin(name.clone()), &mut pin.node)];
            nodes.extend(
                pin.packages
                    .iter_mut()
                    .map(|(package, node)| (NodeKey::PinPackage(name.clone(), package.clone()), node)),
            );
            for (source_name, source) in &mut pin.sources {
                nodes.push((NodeKey::Source(name.clone(), source_name.clone()), &mut source.node));
                nodes.extend(source.packages.iter_mut().map(|(package, node)| {
                    (
                        NodeKey::SourcePackage(name.clone(), source_name.clone(), package.clone()),
                        node,
                    )
                }));
            }
            for (key, node) in nodes {
                if let Some((level, text)) = node.message.take()
                    && let Some(item) = self.items.get(&key)
                {
                    item.message(level, activity_summary(&text));
                }
            }
        }
        self.activity_state = current;
    }

    pub(super) fn information_lines(&self) -> Vec<tui::Line> {
        let (width, height) = self.terminal_dimensions;
        if width < 100 || height < 20 {
            return Vec::new();
        }
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
            tui::Line::Text("Phase:".into()),
            tui::Line::Text(self.phase.unwrap_or(Phase::LoadingConfiguration).label().into()),
            tui::Line::Text(format!("Pins: {completed}/{} · {failed} failed", self.pins.len())),
            tui::Line::Title("Nix".into()),
            tui::Line::Text(format!("Build {builds} · Download {downloads}")),
            tui::Line::Text(format!("Copy {copies} · Query {queries}")),
            tui::Line::Title("Keys".into()),
            tui::Line::Text("j/k scroll".into()),
            tui::Line::Text("q/Esc/Ctrl+C cancel".into()),
        ]
    }

    pub(super) fn visible_trees(&self) -> Vec<TreeNode> {
        self.visible_pin_names()
            .into_iter()
            .filter_map(|name| self.pin_tree(&name))
            .collect()
    }

    pub(super) fn visible_pin_names(&self) -> Vec<String> {
        self.pins.keys().cloned().collect()
    }

    pub(super) fn title_name(&self, visible: usize) -> String {
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

    pub(super) fn pin_tree(&self, name: &str) -> Option<TreeNode> {
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

    pub(super) fn node_for_key(&self, key: &NodeKey) -> Option<(String, Node, usize)> {
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

    pub(super) fn complete_pin(&mut self, name: &str) {
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

#[derive(Clone)]
pub struct Reporter {
    pub(super) state: Arc<Mutex<State>>,
}

impl Reporter {
    pub fn declare(&self, name: &str, current: Option<&str>) {
        self.with_state(|state| {
            state.pins.insert(
                name.into(),
                Pin {
                    node: Node::new("Waiting to check"),
                    current: current.map(str::to_owned),
                    target: None,
                    revision: false,
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

    pub fn revision(&self, name: &str) {
        self.with_pin(name, |pin| pin.revision = true);
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

    pub fn wait(&self, name: &str, reason: &str) {
        self.with_pin(name, |pin| pin.node.wait(reason));
    }

    pub fn sources(&self, name: &str, sources: Vec<String>) {
        self.with_pin(name, |pin| {
            pin.sources.clear();
            pin.node.wait("Waiting to download");
            pin.sources_collapsed = is_default_only(&sources);
            if !pin.sources_collapsed {
                for source in sources {
                    pin.sources.insert(
                        source,
                        Source {
                            node: Node::new("Waiting to download"),
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

    pub fn source_wait(&self, name: &str, source: &str, reason: &str) {
        self.with_pin(name, |pin| {
            if pin.sources_collapsed && source == "default" || pin.sources.is_empty() {
                pin.node.wait(reason);
            } else if let Some(source) = pin.sources.get_mut(source) {
                source.node.wait(reason);
                if pin
                    .sources
                    .values()
                    .all(|source| source.node.status == Status::Waiting && source.node.step.as_deref() == Some(reason))
                {
                    pin.node.wait(reason);
                }
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

    pub(super) fn with_source_packages(
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

    pub(super) fn with_source_packages_update(
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

    pub(super) fn with_pin(&self, name: &str, update: impl FnOnce(&mut Pin)) {
        self.with_state(|state| {
            if let Some(pin) = state.pins.get_mut(name) {
                update(pin);
            }
        });
    }

    /// 声明节点、展开分支或改变终态后重建可见树。
    pub(super) fn with_state(&self, update: impl FnOnce(&mut State)) {
        if let Ok(mut state) = self.state.lock() {
            update(&mut state);
            state.rebuild();
        }
    }

    /// 阶段与计数更新沿用已有节点；回调在状态锁内执行，不应运行外部命令。
    pub(super) fn with_state_update(&self, update: impl FnOnce(&mut State)) {
        if let Ok(mut state) = self.state.lock() {
            update(&mut state);
            state.refresh_items();
        }
    }
}

pub(super) fn is_default_only(names: &[String]) -> bool {
    names.len() == 1 && names[0] == "default"
}

pub(super) fn replace_packages(
    parent: &mut Node,
    packages: &mut BTreeMap<String, Node>,
    collapsed: &mut bool,
    names: Vec<String>,
) {
    packages.clear();
    parent.wait("Waiting to build");
    *collapsed = is_default_only(&names);
    if !*collapsed {
        for name in names {
            packages.insert(name, Node::new("Waiting to build"));
        }
    }
}

pub(super) fn package_step(
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

pub(super) fn package_detail(
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

pub(super) fn finish_package(parent: &mut Node, packages: &mut BTreeMap<String, Node>, collapsed: bool, package: &str) {
    if collapsed && package == "default" {
        parent.clear_activity();
    } else {
        packages.remove(package);
    }
}

pub(super) fn fail_package(
    parent: &mut Node,
    packages: &mut BTreeMap<String, Node>,
    collapsed: bool,
    package: &str,
    key: &str,
) {
    if collapsed && package == "default" {
        parent.fail(Some(key.into()));
    } else if let Some(package) = packages.get_mut(package) {
        package.fail(Some(key.into()));
    }
}

pub(super) fn pin_base(name: &str, pin: &Pin) -> String {
    let (current, target) = display_versions(pin.current.as_deref(), pin.target.as_deref(), pin.revision);
    let version = match (&current, &target) {
        (Some(current), Some(target)) if pin.current != pin.target => format!("{current} → {target}"),
        (Some(current), _) => current.clone(),
        (None, Some(target)) => format!("— → {target}"),
        (None, None) => "—".into(),
    };
    format!("{name} {version}")
}

/// 只缩写被 Checker 标记为 revision 的值；新旧前缀冲突时延长到可区分，其余版本保持原样。
pub(super) fn display_versions(
    current: Option<&str>,
    target: Option<&str>,
    revision: bool,
) -> (Option<String>, Option<String>) {
    let is_rev =
        |value: &str| revision && matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit());
    let mut length = 7;
    if let (Some(current), Some(target)) = (current, target)
        && current != target
        && is_rev(current)
        && is_rev(target)
    {
        while current[..length.min(current.len())] == target[..length.min(target.len())] {
            length = (length + 4).min(current.len().max(target.len()));
        }
    }
    let display = |value: &str| {
        if is_rev(value) {
            value[..length.min(value.len())].to_owned()
        } else {
            value.to_owned()
        }
    };
    (current.map(display), target.map(display))
}

pub(super) fn source_name(key: &NodeKey) -> &str {
    match key {
        NodeKey::Source(_, source) => source,
        _ => "",
    }
}

pub(super) fn collect_tree_keys(tree: &TreeNode, keys: &mut Vec<NodeKey>) {
    keys.push(tree.key.clone());
    for child in &tree.children {
        collect_tree_keys(child, keys);
    }
}

pub(super) fn collect_activities(tree: &TreeNode, activities: &mut BTreeMap<NodeKey, Vec<String>>) {
    if !tree.node.activities.is_empty() {
        activities.insert(tree.key.clone(), tree.node.activities.clone());
    }
    for child in &tree.children {
        collect_activities(child, activities);
    }
}

pub(super) fn count_node_download(node: &Node) -> usize {
    usize::from(matches!(
        node.detail,
        Some(NixProgress::Counter {
            unit: NixProgressUnit::Bytes | NixProgressUnit::Objects,
            ..
        })
    ))
}

pub(super) fn count_pin_downloads(pin: &Pin) -> usize {
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

pub(super) fn materialize_root(root: &Root, tree: &TreeNode, items: &mut BTreeMap<NodeKey, Item>) {
    let mut item = root.add_child("");
    apply_node(&mut item, &tree.base, &tree.node);
    for child in &tree.children {
        materialize_tree(&mut item, child, items);
    }
    items.insert(tree.key.clone(), item);
}

pub(super) fn materialize_tree(parent: &mut Item, tree: &TreeNode, items: &mut BTreeMap<NodeKey, Item>) {
    let mut item = parent.add_child("");
    apply_node(&mut item, &tree.base, &tree.node);
    for child in &tree.children {
        materialize_tree(&mut item, child, items);
    }
    items.insert(tree.key.clone(), item);
}

pub(super) fn apply_node(item: &mut Item, base: &str, node: &Node) {
    let counter = matches!(node.detail, Some(NixProgress::Counter { .. }));
    let active = node.status == Status::Active && !counter;
    item.set_name(format_node_text(base, node, None));
    match &node.detail {
        Some(NixProgress::Counter {
            current, total, unit, ..
        }) => {
            item.init(total.map(saturating_usize), Some(progress_unit(*unit)));
            item.set(saturating_usize(*current));
        }
        _ if active => {
            let step = item.step().unwrap_or(0);
            item.init(None, Some(unit::dynamic(SpinnerUnit)));
            item.set(step);
        }
        _ => item.init(None, None),
    }
}
