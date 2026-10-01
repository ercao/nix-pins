use super::*;
use crosstermion::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use futures_lite::Stream;
use prodash::render::tui as prodash_tui;
pub(super) use prodash_tui::Line;
use std::pin::Pin as FuturePin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::task::{Context, Poll};
use std::thread;

fn tui_options() -> prodash_tui::Options {
    prodash_tui::Options {
        title: "Nix Pins".into(),
        throughput: true,
        recompute_column_width_every_nth_frame: Some(10),
        stop_if_progress_missing: false,
        ..Default::default()
    }
}

struct EventStream {
    receiver: mpsc::Receiver<prodash_tui::Event>,
}

impl Stream for EventStream {
    type Item = prodash_tui::Event;

    fn poll_next(self: FuturePin<&mut Self>, _: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match self.receiver.try_recv() {
            Ok(event) => Poll::Ready(Some(event)),
            Err(mpsc::TryRecvError::Empty) => Poll::Pending,
            Err(mpsc::TryRecvError::Disconnected) => Poll::Ready(None),
        }
    }
}

pub(super) struct TuiRenderer {
    events: mpsc::Sender<prodash_tui::Event>,
    shutting_down: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl TuiRenderer {
    pub(super) fn start(state: &Arc<Mutex<State>>) -> io::Result<Self> {
        let root = state.lock().unwrap().root.clone();
        let (events, receiver) = mpsc::channel();
        let render = prodash_tui::render_with_input(
            io::stderr(),
            Arc::downgrade(&root),
            tui_options(),
            EventStream { receiver },
        )?;
        let shutting_down = Arc::new(AtomicBool::new(false));
        let renderer_stopping = Arc::clone(&shutting_down);
        let handle = thread::spawn(move || {
            futures_lite::future::block_on(render);
            if !renderer_stopping.load(Ordering::SeqCst) {
                crate::process::cancel();
            }
        });
        Ok(Self {
            events,
            shutting_down,
            handle: Some(handle),
        })
    }

    pub(super) fn shutdown(&mut self) {
        self.shutting_down.store(true, Ordering::SeqCst);
        let _ = self.events.send(prodash_tui::Event::Input(KeyEvent::new(
            KeyCode::Char('q'),
            KeyModifiers::NONE,
        )));
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}
