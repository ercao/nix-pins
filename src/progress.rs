//! 终端任务进度。业务线程只发送显示事件，renderer 独占动态 stderr 区域。

use indicatif::{MultiProgress, ProgressBar, ProgressDrawTarget, ProgressStyle};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, IsTerminal};
use std::sync::{mpsc, Arc};
use std::thread::{self, JoinHandle};

#[derive(Clone, Copy)]
pub enum Stage {
    Checking,
    Sources,
    Derived,
    Writing,
}

impl Stage {
    fn label(self) -> &'static str {
        match self {
            Self::Checking => "Checking versions",
            Self::Sources => "Hashing sources",
            Self::Derived => "Hashing derived",
            Self::Writing => "Writing pins.json",
        }
    }
}

enum Event {
    StageStarted { stage: Stage, total: usize },
    TaskStarted { name: String },
    TaskDetail { name: String, detail: String },
    TaskFinished { name: String },
    StageFinished,
    Flush(mpsc::Sender<()>),
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

    pub fn stage<I>(&self, stage: Stage, names: I) -> StageReporter
    where
        I: IntoIterator<Item = String>,
    {
        let names = Arc::new(names.into_iter().collect::<BTreeSet<_>>());
        let reporter = StageReporter {
            sender: self.sender.clone(),
            names,
        };
        if !reporter.names.is_empty() {
            reporter.send(Event::StageStarted {
                stage,
                total: reporter.names.len(),
            });
        }
        reporter
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
pub struct StageReporter {
    sender: Option<mpsc::Sender<Event>>,
    names: Arc<BTreeSet<String>>,
}

impl StageReporter {
    pub fn task_started(&self, name: &str) {
        if self.names.contains(name) {
            self.send(Event::TaskStarted { name: name.into() });
        }
    }

    pub fn detail(&self, name: &str, detail: &str) {
        if self.names.contains(name) {
            self.send(Event::TaskDetail {
                name: name.into(),
                detail: detail.into(),
            });
        }
    }

    pub fn task_finished(&self, name: &str) {
        if self.names.contains(name) {
            self.send(Event::TaskFinished { name: name.into() });
        }
    }

    pub fn finish(self) {
        if self.names.is_empty() {
            return;
        }
        self.send(Event::StageFinished);
        let (done, wait) = mpsc::channel();
        if let Some(sender) = &self.sender {
            if sender.send(Event::Flush(done)).is_ok() {
                let _ = wait.recv();
            }
        }
    }

    fn send(&self, event: Event) {
        if let Some(sender) = &self.sender {
            let _ = sender.send(event);
        }
    }
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
    for event in receiver {
        match event {
            Event::TaskDetail { name, detail } => eprintln!("hashing {name}: {detail}"),
            Event::Flush(done) => {
                let _ = done.send(());
            }
            Event::Finished(done) => {
                let _ = done.send(());
                break;
            }
            _ => {}
        }
    }
}

struct Renderer {
    multi: MultiProgress,
    stage: Option<ProgressBar>,
    tasks: BTreeMap<String, ProgressBar>,
}

impl Renderer {
    fn new(target: ProgressDrawTarget) -> Self {
        Self {
            multi: MultiProgress::with_draw_target(target),
            stage: None,
            tasks: BTreeMap::new(),
        }
    }

    fn apply(&mut self, event: Event) -> bool {
        match event {
            Event::StageStarted { stage, total } => {
                let bar = ProgressBar::new(total as u64);
                bar.set_style(
                    ProgressStyle::with_template("{msg:<18} [{bar:20.cyan/blue}] {pos}/{len}")
                        .unwrap()
                        .progress_chars("=>-"),
                );
                bar.set_message(stage.label());
                let bar = self.multi.add(bar);
                bar.tick();
                self.stage = Some(bar);
            }
            Event::TaskStarted { name } => {
                let spinner = ProgressBar::new_spinner();
                spinner.set_style(
                    ProgressStyle::with_template("{spinner} {msg}")
                        .unwrap()
                        .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏ "),
                );
                spinner.set_message(name.clone());
                let spinner = self.multi.add(spinner);
                spinner.tick();
                self.tasks.insert(name, spinner);
            }
            Event::TaskDetail { name, detail } => {
                if let Some(task) = self.tasks.get(&name) {
                    task.set_message(format!("{name}: {detail}"));
                    task.tick();
                }
            }
            Event::TaskFinished { name } => {
                if let Some(task) = self.tasks.remove(&name) {
                    task.finish_and_clear();
                    self.multi.remove(&task);
                }
                if let Some(stage) = &self.stage {
                    stage.inc(1);
                }
            }
            Event::StageFinished => self.clear(),
            Event::Flush(done) => {
                let _ = done.send(());
            }
            Event::Finished(done) => {
                self.clear();
                let _ = done.send(());
                return false;
            }
        }
        true
    }

    fn clear(&mut self) {
        for (_, task) in std::mem::take(&mut self.tasks) {
            task.finish_and_clear();
            self.multi.remove(&task);
        }
        if let Some(stage) = self.stage.take() {
            stage.finish_and_clear();
            self.multi.remove(&stage);
        }
        let _ = self.multi.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use indicatif::InMemoryTerm;

    #[test]
    fn renderer_shows_active_task_and_clears_finished_stage() {
        let term = InMemoryTerm::new(8, 80);
        let mut renderer = Renderer::new(ProgressDrawTarget::term_like(Box::new(term.clone())));

        renderer.apply(Event::StageStarted {
            stage: Stage::Checking,
            total: 2,
        });
        renderer.apply(Event::TaskStarted {
            name: "ripgrep".into(),
        });

        let active = term.contents();
        assert!(active.contains("Checking versions"), "{active}");
        assert!(active.contains("ripgrep"), "{active}");

        renderer.apply(Event::TaskFinished {
            name: "ripgrep".into(),
        });
        renderer.apply(Event::StageFinished);

        assert_eq!(term.contents(), "");
    }
}
