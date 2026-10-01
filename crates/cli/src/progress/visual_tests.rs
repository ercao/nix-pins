//! 由外部 PTY 脚本按 ready/release 文件协议采集真实渲染帧，默认测试不会运行。

use super::*;
use std::{path::PathBuf, time::Duration};

#[test]
#[ignore = "Captured in a PTY by scripts/tui_visual_check.py"]
fn capture_state() {
    let scene = std::env::var("TUI_SCENE").expect("TUI_SCENE");
    let gate = PathBuf::from(std::env::var_os("TUI_GATE").expect("TUI_GATE"));
    assert!(super::scenes::SCENES.iter().any(|(name, _)| *name == scene));
    let progress = Progress::stderr().unwrap();
    super::scenes::populate(&progress, &scene);
    // 在状态注入完成后通知采集方，保持渲染器运行直到对方抓帧后释放。
    std::fs::write(gate.join("ready"), &scene).unwrap();
    for _ in 0..1000 {
        if gate.join("release").exists() || crate::process::cancelled() {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    progress.finish();
    assert!(!crate::process::cancelled());
}
