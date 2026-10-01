//! 汇总 Pin/Source/Package 的显示状态；TTY 使用全屏视图，非 TTY 只输出低频结果。

mod plain;
mod state;
mod tui;

use crate::nix::{NixProgress, NixProgressUnit};
use crate::nix::{activity_message_level, activity_summary, terminal_text};
use crosstermion::crossterm;
use plain::*;
use prodash::messages::MessageLevel;
use prodash::tree::{Item, Root, root};
use prodash::unit::{self, display::Mode};
use state::*;
pub use state::{Phase, PinStep, Reporter};
use std::collections::BTreeMap;
use std::io::{self, IsTerminal, Write};
use std::sync::{Arc, Mutex};
use tui::TuiRenderer;

fn tui_dimensions_supported((width, height): (u16, u16)) -> bool {
    width >= 60 && height >= 12
}

pub struct Progress {
    state: Arc<Mutex<State>>,
    renderer: Mutex<Option<TuiRenderer>>,
}

impl Progress {
    pub fn stderr() -> Result<Self, ctrlc::Error> {
        crate::process::install_ctrlc_handler()?;
        let dimensions = crossterm::terminal::size().unwrap_or((80, 24));
        let render = io::stderr().is_terminal()
            && tui_dimensions_supported(dimensions)
            && std::env::var_os("NO_COLOR").is_none_or(|value| value.is_empty())
            && std::env::var_os("TERM").is_none_or(|value| value != "dumb");
        Ok(Self::new(Box::new(io::stderr()), dimensions, render, render))
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
        let state = Arc::new(Mutex::new(State::new(
            root,
            SharedWriter::new(output),
            terminal_output,
            terminal_dimensions,
        )));
        let renderer = render.then(|| TuiRenderer::start(&state)).transpose().ok().flatten();
        if render && renderer.is_none() {
            state.lock().unwrap().output_is_terminal = false;
        }
        Self {
            state,
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
        let mut state = self.state.lock().unwrap();
        state.write_snapshot();
        state.output_is_terminal
    }

    /// 先移走渲染器再等待线程，避免持有应用状态锁时与快照线程互相等待。
    fn shutdown(&self) {
        if let Ok(mut renderer) = self.renderer.lock()
            && let Some(mut renderer) = renderer.take()
        {
            renderer.shutdown();
        }
    }
}

impl Drop for Progress {
    fn drop(&mut self) {
        self.shutdown();
    }
}
