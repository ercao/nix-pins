//! 文本输出、结束时的静态树与计数格式，复用同一份显示状态。

use super::*;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

#[derive(Clone)]
pub(super) struct SharedWriter {
    inner: Arc<Mutex<Box<dyn Write + Send>>>,
}

impl SharedWriter {
    pub(super) fn new(writer: Box<dyn Write + Send>) -> Self {
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

pub(super) struct SpinnerUnit;

impl unit::DisplayValue for SpinnerUnit {
    fn display_current_value(&self, _: &mut dyn std::fmt::Write, _: usize, _: Option<usize>) -> std::fmt::Result {
        Ok(())
    }

    fn dyn_hash(&self, _: &mut dyn std::hash::Hasher) {}

    fn display_unit(&self, _: &mut dyn std::fmt::Write, _: usize) -> std::fmt::Result {
        Ok(())
    }
}

pub(super) fn format_node_text(base: &str, node: &Node, counter_values: Option<bool>) -> String {
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
        Some(NixProgress::Status { .. } | NixProgress::Message { .. }) | None => {}
    }
    terminal_text(&text).replace('\n', " ")
}

pub(super) fn render_snapshot_children(output: &mut SharedWriter, children: &[TreeNode], prefix: &str, width: u16) {
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

pub(super) fn truncate_width(text: &str, max_width: usize) -> String {
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

pub(super) fn progress_unit(unit: NixProgressUnit) -> unit::Unit {
    match unit {
        NixProgressUnit::Bytes => {
            unit::dynamic_and_mode(prodash::unit::Bytes, Mode::with_percentage().and_throughput())
        }
        NixProgressUnit::Objects => unit::label_and_mode("objects", Mode::with_percentage().and_throughput()),
    }
}

pub(super) fn saturating_usize(value: u64) -> usize {
    usize::try_from(value).unwrap_or(usize::MAX)
}

#[cfg(test)]
#[derive(Clone)]
pub(super) struct BufferWriter(pub(super) Arc<Mutex<Vec<u8>>>);

#[cfg(test)]
impl Write for BufferWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().write(buffer)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl State {
    pub(super) fn write_snapshot(&mut self) {
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

    pub(super) fn log_pin(&mut self, name: &str) {
        if self.output_is_terminal {
            return;
        }
        if let Some(pin) = self.pins.get(name) {
            let line = format_node_text(&pin_base(name, pin), &pin.node, None);
            let _ = writeln!(self.output, "{line}");
        }
    }

    pub(super) fn log_phase_done(&mut self, phase: Phase) {
        let _ = writeln!(self.output, "{} done", phase.label());
    }
}
