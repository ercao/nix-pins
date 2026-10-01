//! 从共享状态快照绘制任务树和消息，集中管理滚动、取消与终端恢复。

use super::*;
use crossterm::{
    cursor::{Hide, Show},
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use prodash::Root as _;
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line as TextLine, Span},
    widgets::{Block, Gauge, Paragraph},
};
use std::{
    sync::{
        Weak,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
use unicode_width::UnicodeWidthStr;

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Line {
    Title(String),
    Text(String),
}

/// 在状态锁内复制一帧所需数据，实际终端绘制和键盘轮询在锁外进行。
struct Snapshot {
    title: String,
    trees: Vec<TreeNode>,
    information: Vec<Line>,
    messages: Vec<prodash::messages::Message>,
}

impl Snapshot {
    fn new(state: &State) -> Self {
        let mut messages = Vec::new();
        state.root.copy_messages(&mut messages);
        Self {
            title: format!(
                "Nix Pins · {} · {} pins",
                state.phase.unwrap_or(Phase::LoadingConfiguration).label(),
                state.pins.len()
            ),
            trees: state.visible_trees(),
            information: state.information_lines(),
            messages,
        }
    }
}

#[derive(Default)]
struct View {
    tasks_offset: usize,
    messages_offset: usize,
}

fn task_style(status: Status) -> Style {
    let color = match status {
        Status::Waiting => Color::Rgb(148, 163, 184),
        Status::Active => Color::Rgb(115, 200, 208),
        Status::Success => Color::Rgb(106, 207, 157),
        Status::Failure => Color::Rgb(237, 119, 123),
    };
    let style = Style::default().fg(color);
    if status == Status::Failure {
        style.add_modifier(Modifier::BOLD)
    } else {
        style
    }
}

fn draw(frame: &mut Frame, snapshot: &Snapshot, view: &mut View, animation: u128) {
    let area = frame.area();
    let (main, information) = if snapshot.information.is_empty() {
        (area, None)
    } else {
        let [main, info] = Layout::horizontal([Constraint::Min(40), Constraint::Length(32)]).areas(area);
        (main, Some(info))
    };
    let (tasks, messages) = if snapshot.messages.is_empty() {
        (main, None)
    } else {
        let [tasks, messages] =
            Layout::vertical([Constraint::Min(5), Constraint::Length((main.height / 3).clamp(4, 10))]).areas(main);
        (tasks, Some(messages))
    };
    let block = Block::bordered()
        .title(snapshot.title.as_str())
        .title_bottom(" j/k scroll · q/Esc/Ctrl+C cancel ");
    let inner = block.inner(tasks);
    frame.render_widget(block, tasks);
    let mut rows = Vec::new();
    collect_rows(&snapshot.trees, "", &mut rows);
    let right_reserve = (if inner.width >= 100 {
        48
    } else if inner.width >= 70 {
        32
    } else {
        24
    })
    .min(inner.width.saturating_sub(1));
    let content_width = rows
        .iter()
        .map(|(prefix, tree)| UnicodeWidthStr::width(task_identity(prefix, tree, animation).as_str()))
        .max()
        .unwrap_or(0);
    // 按全部可见树节点计算，滚动和 spinner 换帧不会改变分栏位置。
    let left_width = (content_width + 1).min(usize::from(inner.width.saturating_sub(right_reserve + 1))) as u16;
    view.tasks_offset = view
        .tasks_offset
        .min(rows.len().saturating_sub(usize::from(inner.height)));
    for (index, (prefix, tree)) in rows
        .iter()
        .skip(view.tasks_offset)
        .take(usize::from(inner.height))
        .enumerate()
    {
        draw_task(
            frame,
            Rect::new(inner.x, inner.y + index as u16, inner.width, 1),
            prefix,
            tree,
            animation,
            left_width,
        );
    }
    if let Some(area) = information {
        let lines = snapshot
            .information
            .iter()
            .map(|line| match line {
                Line::Title(text) => TextLine::styled(text, Style::default().add_modifier(Modifier::BOLD)),
                Line::Text(text) => TextLine::raw(text),
            })
            .collect::<Vec<_>>();
        frame.render_widget(
            Paragraph::new(lines).block(Block::bordered().title("Information")),
            area,
        );
    }
    if let Some(area) = messages {
        let block = Block::bordered().title("Messages · [/] scroll");
        let height = usize::from(block.inner(area).height);
        view.messages_offset = view.messages_offset.min(snapshot.messages.len().saturating_sub(height));
        let start = snapshot.messages.len().saturating_sub(height + view.messages_offset);
        let lines = snapshot
            .messages
            .iter()
            .skip(start)
            .take(height)
            .map(|message| {
                let (label, color) = match message.level {
                    MessageLevel::Info => (" INFO ", Color::White),
                    MessageLevel::Success => (" DONE ", Color::Green),
                    MessageLevel::Failure => (" FAIL ", Color::Red),
                };
                TextLine::from(vec![
                    Span::styled(label, Style::default().fg(Color::Black).bg(color)),
                    // 正文优先，长任务名称不能挤掉窄屏下的错误诊断。
                    Span::raw(format!(
                        " {} · {}",
                        terminal_text(&message.message),
                        terminal_text(&message.origin)
                    )),
                ])
            })
            .collect::<Vec<_>>();
        frame.render_widget(Paragraph::new(lines).block(block), area);
    }
}

fn collect_rows<'a>(trees: &'a [TreeNode], prefix: &str, out: &mut Vec<(String, &'a TreeNode)>) {
    for (index, tree) in trees.iter().enumerate() {
        let last = index + 1 == trees.len();
        let branch = if prefix.is_empty() {
            String::new()
        } else {
            format!("{prefix}{}", if last { "└─ " } else { "├─ " })
        };
        out.push((branch, tree));
        collect_rows(
            &tree.children,
            &format!("{prefix}{}", if last { "   " } else { "│  " }),
            out,
        );
    }
}

fn task_identity(prefix: &str, tree: &TreeNode, animation: u128) -> String {
    const FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
    let node = &tree.node;
    let icon = if node.status == Status::Active {
        FRAMES[(animation / 100 % 10) as usize]
    } else {
        node.status.symbol()
    };
    let base = terminal_text(&tree.base).replace('\n', " ");
    format!("{prefix}{icon} {base}")
}

/// 总量未知或为零时只显示累计量；已知总量且空间足够时才显示百分比条。
fn draw_task(frame: &mut Frame, area: Rect, prefix: &str, tree: &TreeNode, animation: u128, left_width: u16) {
    let right_width = area.width.saturating_sub(left_width + 1).min(48);
    let left = Rect::new(area.x, area.y, left_width, 1);
    let right = Rect::new(area.x + left_width + 1, area.y, right_width, 1);
    let node = &tree.node;
    let style = task_style(node.status);
    let identity = truncate_width(&task_identity(prefix, tree, animation), usize::from(left.width));
    frame.render_widget(Paragraph::new(identity).style(style), left);
    frame.render_widget("│", Rect::new(area.x + left_width, area.y, 1, 1));
    match &node.detail {
        Some(NixProgress::Counter {
            current,
            total,
            unit,
            label,
        }) => {
            let total = total.filter(|total| *total > 0);
            let values = progress_unit(*unit)
                .display(saturating_usize(*current), total.map(saturating_usize), None)
                .to_string();
            let summary = if right.width >= 40 {
                format!("{} {values}", terminal_text(label).replace('\n', " "))
            } else {
                values
            };
            let text_width = UnicodeWidthStr::width(summary.as_str()) as u16;
            if let Some(total) = total.filter(|_| text_width + 5 <= right.width) {
                frame.render_widget(
                    Paragraph::new(summary).style(style),
                    Rect::new(right.x, right.y, text_width, 1),
                );
                let bar = Rect::new(right.x + text_width + 1, right.y, right.width - text_width - 1, 1);
                frame.render_widget(
                    Gauge::default()
                        .ratio((*current as f64 / total as f64).clamp(0.0, 1.0))
                        .label("")
                        .gauge_style(style),
                    bar,
                );
            } else {
                frame.render_widget(
                    Paragraph::new(truncate_width(&summary, usize::from(right.width))).style(style),
                    right,
                );
            }
        }
        _ => {
            let mut stage = node.step.clone().unwrap_or_else(|| match node.status {
                Status::Success => "Done".into(),
                Status::Failure => "Failed".into(),
                _ => String::new(),
            });
            if let Some(NixProgress::Text(detail)) = &node.detail {
                if !stage.is_empty() && stage != *detail {
                    stage.push_str(" · ");
                }
                if stage != *detail {
                    stage.push_str(detail);
                }
            }
            let stage = terminal_text(&stage).replace('\n', " ");
            frame.render_widget(
                Paragraph::new(truncate_width(&stage, usize::from(right.width))).style(style),
                right,
            );
        }
    }
}

/// 以作用域守卫恢复终端，覆盖正常退出、初始化失败和渲染线程 panic。
struct TerminalSession(SharedWriter);

impl Drop for TerminalSession {
    fn drop(&mut self) {
        let _ = crossterm::execute!(self.0, LeaveAlternateScreen, Show);
        let _ = disable_raw_mode();
    }
}

pub(super) struct TuiRenderer {
    stopping: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl TuiRenderer {
    pub(super) fn start(state: &Arc<Mutex<State>>) -> io::Result<Self> {
        let output = state.lock().unwrap().output.clone();
        let mut session = TerminalSession(output.clone());
        enable_raw_mode()?;
        crossterm::execute!(session.0, EnterAlternateScreen, Hide)?;
        let mut terminal = Terminal::new(CrosstermBackend::new(output))?;
        // 首帧写入失败也需初始化输入源，避免 macOS 退出时留下 PENDIN 标志。
        event::poll(Duration::ZERO)?;
        let stopping = Arc::new(AtomicBool::new(false));
        let thread_stopping = stopping.clone();
        let weak = Arc::downgrade(state);
        let handle = thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                run(&mut terminal, &weak, &thread_stopping)
            }));
            // 在写入 plain 降级输出之前先恢复终端，避免摘要留在 alternate screen。
            drop(session);
            if !matches!(result, Ok(Ok(())))
                && let Some(state) = weak.upgrade()
            {
                state.lock().unwrap().fallback();
            }
        });
        Ok(Self {
            stopping,
            handle: Some(handle),
        })
    }

    pub(super) fn shutdown(&mut self) {
        self.stopping.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// 渲染器保存状态的弱引用以避免所有权环，逐帧临时取得状态；滚动位置独立于任务状态。
fn run(
    terminal: &mut Terminal<CrosstermBackend<SharedWriter>>,
    weak: &Weak<Mutex<State>>,
    stopping: &AtomicBool,
) -> io::Result<()> {
    let mut view = View::default();
    let start = Instant::now();
    while !stopping.load(Ordering::SeqCst) && !crate::process::cancelled() {
        let Some(state) = weak.upgrade() else { break };
        let snapshot = {
            let mut state = state.lock().unwrap();
            let dimensions = crossterm::terminal::size()?;
            state.resize(dimensions);
            if !tui_dimensions_supported(dimensions) {
                return Err(io::Error::other("terminal is too small"));
            }
            Snapshot::new(&state)
        };
        terminal.draw(|frame| draw(frame, &snapshot, &mut view, start.elapsed().as_millis()))?;
        if event::poll(Duration::from_millis(100))?
            && let Event::Key(key) = event::read()?
        {
            if key.kind == KeyEventKind::Release {
                continue;
            }
            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => crate::process::cancel(),
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => crate::process::cancel(),
                KeyCode::Char('j') | KeyCode::Down => view.tasks_offset = view.tasks_offset.saturating_add(1),
                KeyCode::Char('k') | KeyCode::Up => view.tasks_offset = view.tasks_offset.saturating_sub(1),
                KeyCode::Char('[') => view.messages_offset = view.messages_offset.saturating_add(1),
                KeyCode::Char(']') => view.messages_offset = view.messages_offset.saturating_sub(1),
                _ => (),
            }
        }
    }
    Ok(())
}

#[cfg(test)]
pub(super) fn test_frame(progress: &Progress, dimensions: (u16, u16), animation: u128) -> ratatui::buffer::Buffer {
    let mut state = progress.state.lock().unwrap();
    state.resize(dimensions);
    let snapshot = Snapshot::new(&state);
    drop(state);
    let mut terminal = Terminal::new(ratatui::backend::TestBackend::new(dimensions.0, dimensions.1)).unwrap();
    terminal
        .draw(|frame| draw(frame, &snapshot, &mut View::default(), animation))
        .unwrap();
    terminal.backend().buffer().clone()
}

#[cfg(test)]
pub(super) fn frame_text(buffer: &ratatui::buffer::Buffer) -> String {
    buffer
        .content
        .chunks(usize::from(buffer.area.width))
        .map(|row| {
            let mut text = String::new();
            let mut index = 0;
            while index < row.len() {
                let symbol = row[index].symbol();
                text.push_str(symbol);
                index += UnicodeWidthStr::width(symbol).max(1);
            }
            text
        })
        .collect::<Vec<_>>()
        .join("\n")
}
