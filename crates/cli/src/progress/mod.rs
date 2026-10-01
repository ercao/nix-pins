//! 汇总 Pin/Source/Package 的显示状态；TTY 使用全屏视图，非 TTY 只输出低频结果。

mod plain;
mod state;
mod tui;

#[cfg(test)]
use crate::nix::sanitize_activity;
use crate::nix::{NixProgress, NixProgressUnit};
use crate::nix::{activity_message_level, activity_summary, terminal_text};
use plain::*;
use prodash::messages::MessageLevel;
use prodash::tree::{Item, Root, root};
use prodash::unit::{self, display::Mode};
use state::*;
pub use state::{Phase, PinStep, Reporter};
use std::collections::BTreeMap;
use std::io::{self, IsTerminal, Write};
use std::sync::{Arc, Mutex};
#[cfg(test)]
use std::thread;
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

#[cfg(test)]
mod visual_tests;

#[cfg(test)]
mod scenes;

#[cfg(test)]
mod tests {
    use super::*;

    fn output(output: &Arc<Mutex<Vec<u8>>>) -> String {
        String::from_utf8(output.lock().unwrap().clone()).unwrap()
    }

    #[test]
    fn task_view_identity_column_fits_the_widest_task() {
        let (progress, _) = Progress::test_terminal((160, 34));
        let reporter = progress.reporter();
        reporter.declare("bat", Some("v0.24.0"));
        reporter.declare("forge", Some("7ae13f0111111111111111111111111111111111"));
        reporter.revision("forge");
        reporter.step("bat", PinStep::Checking);
        reporter.step("forge", PinStep::Checking);
        reporter.declare("ghost", None);
        for dimensions in [(80, 24), (160, 34)] {
            let frame = tui::test_frame(&progress, dimensions, 0);
            let text = tui::frame_text(&frame);
            for name in ["bat", "forge", "ghost"] {
                let y = text.lines().position(|line| line.contains(name)).unwrap();
                let row = &frame.content[y * usize::from(dimensions.0)..(y + 1) * usize::from(dimensions.0)];
                let divider = row
                    .iter()
                    .enumerate()
                    .skip(1)
                    .find(|(_, cell)| cell.symbol() == "│")
                    .unwrap()
                    .0;
                // 最长内容为 15 列，留 1 列间距，外边框占 1 列。
                assert_eq!(divider, 17, "{name} at {dimensions:?}");
            }
        }
    }

    #[test]
    fn task_view_separates_identity_from_stage_and_progress() {
        let (progress, _) = Progress::test_terminal((160, 34));
        let reporter = progress.reporter();
        reporter.declare("demo", Some("v1"));
        reporter.target("demo", "v2");
        reporter.detail(
            "demo",
            Some(NixProgress::Counter {
                current: 12_000_000,
                total: Some(20_000_000),
                unit: NixProgressUnit::Bytes,
                label: "Downloading".into(),
            }),
        );
        let frame = tui::test_frame(&progress, (160, 34), 0);
        let text = tui::frame_text(&frame);
        let row = text.lines().find(|line| line.contains("demo v1 → v2")).unwrap();
        let (identity, activity) = row.trim_start_matches('│').split_once('│').unwrap();
        assert!(identity.contains("demo v1 → v2"));
        assert!(!identity.contains("Downloading"));
        assert!(activity.contains("Downloading"));
        assert!(activity.contains("60%"));
        assert!(activity.contains("12.0MB/20.0MB"));
    }

    #[test]
    fn task_view_unknown_total_has_no_bar_and_preserves_status_colors() {
        use ratatui::style::Color;
        let (progress, _) = Progress::test_terminal((160, 34));
        let reporter = progress.reporter();
        reporter.declare("active", None);
        reporter.detail(
            "active",
            Some(NixProgress::Counter {
                current: 12_000_000,
                total: None,
                unit: NixProgressUnit::Bytes,
                label: "Downloading".into(),
            }),
        );
        reporter.declare("waiting", None);
        reporter.declare("done", None);
        reporter.done("done", None);
        reporter.declare("failed", None);
        reporter.failed("failed", "npmDepsHash");
        let frame = tui::test_frame(&progress, (160, 34), 0);
        let text = tui::frame_text(&frame);
        let row = text.lines().find(|line| line.contains("active")).unwrap();
        assert!(row.contains("12.0MB"));
        assert!(!row.contains('%') && !row.contains('█') && !row.contains('▄'));
        for (icon, color) in [
            ("⠋", Color::Rgb(115, 200, 208)),
            ("○", Color::Rgb(148, 163, 184)),
            ("✔", Color::Rgb(106, 207, 157)),
            ("⚠", Color::Rgb(237, 119, 123)),
        ] {
            let cell = frame.content.iter().find(|cell| cell.symbol() == icon).unwrap();
            assert_eq!(cell.fg, color);
        }
        let narrow = tui::frame_text(&tui::test_frame(&progress, (60, 12), 0));
        for expected in ["active", "waiting", "done", "failed", "12.0MB", "npmDepsHash"] {
            assert!(narrow.contains(expected), "{expected}\n{narrow}");
        }
    }

    #[test]
    fn task_view_zero_total_does_not_invent_a_percentage() {
        let (progress, _) = Progress::test_terminal((160, 34));
        let reporter = progress.reporter();
        reporter.declare("zero-total", None);
        reporter.detail(
            "zero-total",
            Some(NixProgress::Counter {
                current: 10,
                total: Some(0),
                unit: NixProgressUnit::Bytes,
                label: "Downloading".into(),
            }),
        );
        let frame = tui::frame_text(&tui::test_frame(&progress, (160, 34), 0));
        let row = frame.lines().find(|line| line.contains("zero-total")).unwrap();
        assert!(!row.contains('%') && !row.contains('█'), "{row}");
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
        assert_eq!(truncate_width("✔ ＷＩ-example", 8), "✔ ＷＩ-…");
    }

    #[test]
    fn waiting_clears_activity_and_keeps_reason() {
        let (progress, output_buffer) = Progress::test_terminal((80, 24));
        progress.phase(Phase::CheckingVersions);
        let reporter = progress.reporter();
        reporter.declare("demo", Some("v1"));
        reporter.step("demo", PinStep::Checking);
        reporter.detail("demo", Some(NixProgress::Text("old activity".into())));
        reporter.wait("demo", "Waiting to download");
        progress.complete();
        progress.finish();

        assert_eq!(
            output(&output_buffer),
            "Pin Progress · done\n└─ ○ demo v1 · Waiting to download\n"
        );
    }

    #[test]
    fn waiting_reasons_follow_source_and_package_stages() {
        let (progress, _) = Progress::test_terminal((160, 34));
        let reporter = progress.reporter();
        reporter.declare("demo", Some("v1"));
        assert_eq!(
            progress.state.lock().unwrap().pins["demo"].node.step.as_deref(),
            Some("Waiting to check")
        );
        reporter.wait("demo", "Waiting to download");
        reporter.sources("demo", vec!["api".into(), "web".into()]);
        reporter.source_step("demo", "api", PinStep::HashingSource);
        reporter.source_wait("demo", "api", "Waiting to build");
        {
            let state = progress.state.lock().unwrap();
            let pin = &state.pins["demo"];
            assert_eq!(pin.node.step.as_deref(), Some("Waiting to download"));
            assert_eq!(pin.sources["api"].node.step.as_deref(), Some("Waiting to build"));
            assert_eq!(pin.sources["web"].node.step.as_deref(), Some("Waiting to download"));
        }
        reporter.source_wait("demo", "web", "Waiting to build");
        reporter.source_packages("demo", "web", vec!["frontend".into(), "server".into()]);
        {
            let state = progress.state.lock().unwrap();
            let pin = &state.pins["demo"];
            assert_eq!(pin.node.step.as_deref(), Some("Waiting to build"));
            assert_eq!(
                pin.sources["web"].packages["frontend"].step.as_deref(),
                Some("Waiting to build")
            );
        }
        reporter.source_done("demo", "api");
        reporter.source_done("demo", "web");
        let state = progress.state.lock().unwrap();
        assert_eq!(state.pins["demo"].node.status, Status::Success);
        assert!(state.pins["demo"].node.step.is_none());
    }

    #[test]
    fn last_source_finishes_pin_immediately() {
        let (progress, _) = Progress::test_terminal((80, 24));
        progress.phase(Phase::ProcessingPins);
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
        progress.phase(Phase::ProcessingPins);
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
    fn warnings_keep_their_label_without_becoming_failures() {
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
        assert_eq!(messages[0].level, MessageLevel::Info);
        assert_eq!(messages[0].message, "warning: cache is stale");
    }

    #[test]
    fn information_summarizes_progress_and_active_nix_work() {
        let (progress, _) = Progress::test_terminal((100, 30));
        progress.phase(Phase::ProcessingPins);
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
                tui::Line::Text("Phase:".into()),
                tui::Line::Text("Processing pins".into()),
                tui::Line::Text("Pins: 1/3 · 0 failed".into()),
                tui::Line::Title("Nix".into()),
                tui::Line::Text("Build 1 · Download 2".into()),
                tui::Line::Text("Copy 0 · Query 1".into()),
                tui::Line::Title("Keys".into()),
                tui::Line::Text("j/k scroll".into()),
                tui::Line::Text("q/Esc/Ctrl+C cancel".into()),
            ]
        );
    }

    #[test]
    fn revision_display_preserves_tags_and_extends_collisions() {
        let current = "1234567aaaa11111111111111111111111111111";
        let target = "1234567bbbb22222222222222222222222222222";
        assert_eq!(
            display_versions(Some(current), Some(target), false),
            (Some(current.into()), Some(target.into()))
        );
        assert_eq!(
            display_versions(Some(current), Some(target), true),
            (Some("1234567aaaa".into()), Some("1234567bbbb".into()))
        );
        assert_eq!(
            display_versions(Some(current), Some(current), true),
            (Some("1234567".into()), Some("1234567".into()))
        );
        assert_eq!(
            display_versions(None, Some(target), true),
            (None, Some("1234567".into()))
        );
        assert_eq!(
            display_versions(Some("v1"), Some("v2"), true),
            (Some("v1".into()), Some("v2".into()))
        );
        let (progress, _) = Progress::test_terminal((160, 34));
        let reporter = progress.reporter();
        reporter.declare("demo", Some(current));
        reporter.revision("demo");
        reporter.target("demo", target);
        let state = progress.state.lock().unwrap();
        assert_eq!(pin_base("demo", &state.pins["demo"]), "demo 1234567aaaa → 1234567bbbb");
        assert_eq!(state.pins["demo"].target.as_deref(), Some(target));
    }

    #[test]
    fn display_snapshot_resizes_without_truncating_message_origins() {
        let (progress, _) = Progress::test_terminal((160, 34));
        let reporter = progress.reporter();
        let name = "ＷＩＤＥ-with-a-very-long-name-that-exceeds-the-available-terminal-columns";
        reporter.declare(name, None);
        reporter.step(name, PinStep::HashingSource);
        reporter.detail(
            name,
            Some(NixProgress::Message {
                level: MessageLevel::Failure,
                text: "error: failed".into(),
            }),
        );
        let wide = tui::frame_text(&tui::test_frame(&progress, (160, 34), 0));
        assert!(wide.contains(name));
        assert!(wide.contains("Information"));
        let narrow = tui::frame_text(&tui::test_frame(&progress, (60, 12), 0));
        assert!(narrow.contains("Ｗ"));
        assert!(!narrow.contains("Information"));
        assert!(narrow.contains("error: failed"), "{narrow}");
        let mut messages = Vec::new();
        prodash::Root::copy_messages(&progress.state.lock().unwrap().root, &mut messages);
        assert!(messages[0].origin.contains(name));
        assert!(messages[0].message.contains("error: failed"));
    }

    #[test]
    fn animated_icons_only_change_active_render_names() {
        let (progress, _) = Progress::test_terminal((160, 34));
        let reporter = progress.reporter();
        reporter.declare("checking", None);
        reporter.step("checking", PinStep::Checking);
        reporter.declare("download", None);
        reporter.detail(
            "download",
            Some(NixProgress::Counter {
                current: 10,
                total: Some(20),
                unit: NixProgressUnit::Bytes,
                label: "Downloading".into(),
            }),
        );
        reporter.declare("waiting", None);
        reporter.declare("done", None);
        reporter.done("done", None);
        reporter.declare("failed", None);
        reporter.failed("failed", "Checking");
        let first = tui::frame_text(&tui::test_frame(&progress, (160, 34), 0));
        let second = tui::frame_text(&tui::test_frame(&progress, (160, 34), 100));
        assert!(first.contains("⠋ checking"));
        assert!(second.contains("⠙ checking"));
        assert!(first.contains("⠋ download") && second.contains("⠙ download"));
        for expected in ["○ waiting", "✔ done", "⚠ failed", "50%"] {
            assert!(first.contains(expected) && second.contains(expected), "{expected}");
        }
        reporter.wait("checking", "Waiting to download");
        let waiting = tui::frame_text(&tui::test_frame(&progress, (160, 34), 100));
        assert!(waiting.contains("○ checking") && waiting.contains("Waiting to download"));
    }

    #[test]
    fn renderer_fallback_keeps_results_and_continues_plain_output() {
        let (progress, output_buffer) = Progress::test_terminal((120, 24));
        progress.phase(Phase::CheckingVersions);
        let reporter = progress.reporter();
        reporter.declare("done", Some("v1"));
        reporter.done("done", None);
        reporter.declare("active", None);
        progress.state.lock().unwrap().fallback();
        reporter.done("active", None);
        progress.phase(Phase::WritingPinsFile);
        progress.complete();
        assert!(!progress.finish());
        assert_eq!(
            output(&output_buffer),
            "✔ done v1\n✔ active —\nChecking versions done\nWriting pins.json done\n"
        );
    }

    #[test]
    #[ignore = "Requires a real PTY and an injected render write failure"]
    fn renderer_error_continues_plain_without_cancelling() {
        struct FailOnce(usize);
        impl Write for FailOnce {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                self.0 += 1;
                if self.0 == 100 {
                    return Err(io::Error::other("injected rendering failure"));
                }
                io::stderr().write(bytes)
            }
            fn flush(&mut self) -> io::Result<()> {
                io::stderr().flush()
            }
        }
        crate::process::reset_cancelled();
        let progress = Progress::new(Box::new(FailOnce(0)), (120, 24), true, true);
        assert!(progress.renderer.lock().unwrap().is_some());
        progress.phase(Phase::CheckingVersions);
        let reporter = progress.reporter();
        reporter.declare("demo", None);
        reporter.step("demo", PinStep::Checking);
        for _ in 0..100 {
            if !progress.state.lock().unwrap().output_is_terminal {
                break;
            }
            thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(!progress.state.lock().unwrap().output_is_terminal);
        assert!(!crate::process::cancelled());
        reporter.done("demo", None);
        progress.phase(Phase::WritingPinsFile);
        progress.complete();
        assert!(!progress.finish());
    }

    #[test]
    fn control_sequences_do_not_reach_messages_or_failure_reports() {
        assert_eq!(
            terminal_text("\u{1b}[31merror:\u{1b}[0m first\n    trace\tfile\u{7}"),
            "error: first\n    trace    file"
        );
        assert_eq!(terminal_text("first\rsecond"), "second");
        assert_eq!(
            terminal_text("\u{1b}]8;;https://secret.invalid\u{1b}\\label\u{1b}]8;;\u{1b}\\"),
            "label"
        );
    }

    #[test]
    fn multiline_diagnostics_are_summarized_at_the_feed_boundary() {
        let (progress, _) = Progress::test_terminal((160, 34));
        let reporter = progress.reporter();
        reporter.declare("demo", None);
        reporter.detail("demo", Some(NixProgress::Message {
            level: MessageLevel::Failure,
            text: "… while evaluating\n  at fixture.nix:2:3\n\u{1b}[31merror:\u{1b}[0m download failed\n  context line".into(),
        }));
        let mut messages = Vec::new();
        progress.state.lock().unwrap().root.copy_messages(&mut messages);
        assert_eq!(messages[0].message, "error: download failed");
        assert_eq!(messages[0].level, MessageLevel::Failure);
    }
}
