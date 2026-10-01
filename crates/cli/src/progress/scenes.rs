//! TUI 示例与视觉验证的固定输入，使用真实进度接口构造状态而不执行更新流水线。

use crate::nix::{NixProgress, NixProgressUnit};
use crate::progress::{Phase, PinStep, Progress};
use prodash::messages::MessageLevel;

pub const SCENES: &[(&str, &str)] = &[
    ("all", "All states and layouts"),
    ("status-colors", "Status colors and task tree"),
    ("checking-32", "32 tasks and scrolling"),
    ("S01", "Loading configuration"),
    ("S02", "Checking versions"),
    ("S03", "Target versions and queued downloads"),
    ("S04", "Downloads with known totals"),
    ("S05", "Git object counts"),
    ("S06", "Unknown totals"),
    ("S07", "Applying patches"),
    ("S08", "Waiting to build"),
    ("S09", "Source reuse"),
    ("S10", "Derived hash reuse"),
    ("S11", "Derived hashes and packages"),
    ("S12", "Version check failure"),
    ("S13", "Source failure"),
    ("S14", "Package failure"),
    ("S15", "Writing pins file"),
    ("success", "All pins successful"),
    ("S21", "Long tags and revision collisions"),
    ("S22", "Build activities and messages"),
    ("S23", "Success messages"),
    ("S24", "Build failure messages"),
    ("S26", "Multiline diagnostic summaries"),
];

const OLD_REV: &str = "7ae13f0111111111111111111111111111111111";
const NEW_REV: &str = "9bc24a1222222222222222222222222222222222";

fn counter(current: u64, total: Option<u64>, unit: NixProgressUnit) -> Option<NixProgress> {
    Some(NixProgress::Counter {
        current,
        total,
        unit,
        label: "Downloading".into(),
    })
}

/// 通过生产 Reporter 注入固定场景，供示例与 PTY 采集共用；不启动 Nix 或写入结果文件。
pub fn populate(progress: &Progress, scene: &str) {
    if scene == "all" {
        populate(progress, "status-colors");
        populate_all(progress);
        return;
    }
    let reporter = progress.reporter();
    progress.phase(Phase::LoadingConfiguration);
    if scene != "S01" {
        reporter.declare("bat", Some("v0.24.0"));
        reporter.declare("forge", Some(OLD_REV));
        reporter.revision("forge");
        reporter.declare("ghost", None);
        progress.phase(Phase::CheckingVersions);
    }
    if !matches!(scene, "S01" | "S02" | "S12" | "checking-32") {
        reporter.target("bat", "v0.25.0");
        reporter.target("forge", NEW_REV);
        reporter.target("ghost", NEW_REV);
        reporter.revision("ghost");
        for name in ["bat", "forge", "ghost"] {
            reporter.wait(name, "Waiting to download");
        }
        progress.phase(Phase::ProcessingPins);
    }
    if matches!(
        scene,
        "S04" | "S07" | "S08" | "S09" | "S10" | "S11" | "S13" | "S14" | "S22" | "S23" | "S24" | "S26"
    ) {
        reporter.sources("bat", vec!["default".into()]);
        reporter.sources("forge", vec!["api".into(), "web".into()]);
    }
    match scene {
        "S01" | "S03" => {}
        "S02" => {
            reporter.step("bat", PinStep::Checking);
            reporter.step("forge", PinStep::Checking);
        }
        "checking-32" => {
            for index in 0..29 {
                let name = format!("pin-{index:02}");
                reporter.declare(&name, Some(OLD_REV));
                reporter.revision(&name);
                if index < 6 {
                    reporter.step(&name, PinStep::Checking);
                }
            }
            reporter.step("bat", PinStep::Checking);
            reporter.step("forge", PinStep::Checking);
        }
        "S04" => {
            reporter.source_detail(
                "bat",
                "default",
                counter(12_000_000, Some(20_000_000), NixProgressUnit::Bytes),
            );
            reporter.source_detail(
                "forge",
                "api",
                counter(8_000_000, Some(16_000_000), NixProgressUnit::Bytes),
            );
            reporter.source_step("forge", "web", PinStep::HashingSource);
        }
        "S05" => {
            reporter.detail("bat", counter(64, Some(100), NixProgressUnit::Objects));
            reporter.detail("forge", counter(2, Some(8), NixProgressUnit::Objects));
        }
        "S06" => {
            reporter.detail("bat", counter(12_000_000, None, NixProgressUnit::Bytes));
            reporter.detail("forge", Some(NixProgress::Text("Building".into())));
        }
        "S07" => {
            reporter.source_step("bat", "default", PinStep::PatchingSource);
            reporter.source_step("forge", "web", PinStep::PatchingSource);
        }
        "S08" => {
            reporter.source_step("bat", "default", PinStep::SourceReady);
            reporter.source_step("forge", "web", PinStep::SourceReady);
            reporter.source_wait("bat", "default", "Waiting to build");
            reporter.source_wait("forge", "web", "Waiting to build");
        }
        "S09" => {
            reporter.source_step("bat", "default", PinStep::SourceReused);
            reporter.source_done("forge", "api");
            reporter.source_step("forge", "web", PinStep::HashingSource);
        }
        "S10" => {
            progress.phase(Phase::ProcessingPins);
            reporter.step("bat", PinStep::DerivedReused("vendorHash".into()));
            reporter.source_packages("forge", "web", vec!["frontend".into(), "server".into()]);
            reporter.source_package_reused("forge", "web", "server", "vendorHash");
        }
        "S11" | "S14" | "S22" | "S23" | "S24" | "S26" => {
            progress.phase(Phase::ProcessingPins);
            reporter.source_packages("forge", "web", vec!["frontend".into(), "server".into()]);
            reporter.source_package_active("forge", "web", "frontend", "npmDepsHash");
            reporter.source_package_ready("forge", "web", "server", "vendorHash");
            reporter.source_done("forge", "api");
            reporter.step("bat", PinStep::HashingDerived("vendorHash".into()));
            if scene == "S14" {
                reporter.source_package_failed("forge", "web", "frontend", "npmDepsHash");
                reporter.source_failed("forge", "web");
                reporter.failed("forge", "Source web/Package frontend/Derived Hash npmDepsHash");
            } else if scene != "S11" {
                reporter.source_package_detail(
                    "forge",
                    "web",
                    "frontend",
                    Some(NixProgress::Status {
                        detail: Some(Box::new(NixProgress::Text("buildPhase".into()))),
                        activities: vec!["Building /nix/store/example-frontend.drv · buildPhase".into()],
                    }),
                );
                let (level, text) = match scene {
                    "S23" => (MessageLevel::Success, "Expected hash resolved: sha256-example"),
                    "S24" => (MessageLevel::Failure, "error: dependency download timed out"),
                    "S26" => (
                        MessageLevel::Failure,
                        "… while evaluating\n  at fixture.nix:2:3\nerror: dependency download timed out\n  context line",
                    ),
                    _ => (MessageLevel::Info, "Running npm install"),
                };
                reporter.source_package_detail(
                    "forge",
                    "web",
                    "frontend",
                    Some(NixProgress::Message {
                        level,
                        text: text.into(),
                    }),
                );
                if matches!(scene, "S24" | "S26") {
                    reporter.source_package_failed("forge", "web", "frontend", "npmDepsHash");
                    reporter.source_failed("forge", "web");
                    reporter.failed("forge", "Source web/Package frontend/Derived Hash npmDepsHash");
                }
            }
        }
        "S12" => {
            reporter.step("bat", PinStep::Checking);
            reporter.failed("forge", "Checking");
        }
        "S13" => {
            reporter.source_done("forge", "api");
            reporter.source_failed("forge", "web");
            reporter.failed("forge", "Source web/Fetching source");
            reporter.source_step("bat", "default", PinStep::HashingSource);
        }
        "S15" | "success" => {
            for name in ["bat", "forge", "ghost"] {
                reporter.done(name, None);
            }
            if scene == "success" {
                progress.complete();
            } else {
                progress.phase(Phase::WritingPinsFile);
            }
        }
        "S21" => {
            reporter.declare("hex-tag", Some(OLD_REV));
            reporter.target("hex-tag", NEW_REV);
            reporter.declare("collision", Some("1234567aaaa11111111111111111111111111111"));
            reporter.revision("collision");
            reporter.target("collision", "1234567bbbb22222222222222222222222222222");
        }
        "status-colors" => {
            reporter.wait("bat", "Waiting to build");
            reporter.detail("forge", counter(12_000_000, Some(20_000_000), NixProgressUnit::Bytes));
            reporter.failed("ghost", "Checking");
            reporter.declare("completed", Some("v1"));
            reporter.done("completed", None);
            reporter.declare("checking", None);
            reporter.step("checking", PinStep::Checking);
            reporter.declare("nested", Some("v1"));
            reporter.target("nested", "v2");
            reporter.sources("nested", vec!["api".into(), "web".into()]);
            reporter.source_done("nested", "api");
            reporter.source_packages("nested", "web", vec!["frontend".into(), "server".into()]);
            reporter.source_package_active("nested", "web", "frontend", "npmDepsHash");
            reporter.source_package_failed("nested", "web", "server", "vendorHash");
        }
        _ => panic!("unknown scene: {scene}"),
    }
}

fn populate_all(progress: &Progress) {
    let reporter = progress.reporter();
    for name in [
        "check-queue",
        "download-queue",
        "git-objects",
        "unknown-total",
        "patch-source",
        "reused-source",
        "resolve-source",
        "resolve-derived",
        "derived-hash",
    ] {
        reporter.declare(name, Some("v1"));
        reporter.target(name, "v2");
    }
    reporter.wait("download-queue", "Waiting to download");
    reporter.detail("git-objects", counter(64, Some(100), NixProgressUnit::Objects));
    reporter.detail("unknown-total", counter(12_000_000, None, NixProgressUnit::Bytes));
    for (name, step) in [
        ("patch-source", PinStep::PatchingSource),
        ("reused-source", PinStep::SourceReused),
        ("resolve-source", PinStep::ResolvingSources),
        ("resolve-derived", PinStep::ResolvingDerivedHashes),
        ("derived-hash", PinStep::HashingDerived("vendorHash".into())),
    ] {
        reporter.step(name, step);
    }
    reporter.detail(
        "derived-hash",
        Some(NixProgress::Status {
            detail: Some(Box::new(NixProgress::Text("buildPhase".into()))),
            activities: vec![
                "Building /nix/store/example.drv · buildPhase".into(),
                "Copying /nix/store/example".into(),
                "Querying Cache".into(),
            ],
        }),
    );
    for (name, level, text) in [
        (
            "checking",
            MessageLevel::Info,
            "Layout example: fixed progress data with animated active icons",
        ),
        (
            "completed",
            MessageLevel::Success,
            "Expected hash resolved: sha256-example",
        ),
        (
            "ghost",
            MessageLevel::Failure,
            "error: example build failed; full diagnostics are available in Messages",
        ),
    ] {
        reporter.detail(
            name,
            Some(NixProgress::Message {
                level,
                text: text.into(),
            }),
        );
    }
}
