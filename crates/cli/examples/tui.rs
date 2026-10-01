//! 在真实终端预览固定场景，复用生产渲染器而不执行下载、求值或 Pins File 保存。

use clap::Parser;
use std::{
    io::{self, IsTerminal},
    thread,
    time::Duration,
};

// 与生产命令复用同一渲染器及状态类型，示例仅注入场景数据，不执行外部命令。
#[allow(dead_code, unused_imports)]
#[path = "../src/nix/mod.rs"]
mod nix;
#[allow(dead_code)]
#[path = "../src/process.rs"]
mod process;
#[allow(dead_code)]
#[path = "../src/progress/mod.rs"]
mod progress;
#[path = "../src/progress/scenes.rs"]
mod scenes;

#[derive(Parser)]
#[command(about = "TUI states and layouts: j/k to scroll, q/Esc/Ctrl+C to quit")]
struct Args {
    #[arg(
        long,
        default_value = "all",
        help = "Select a scene; use --list to see available scenes"
    )]
    scene: String,
    #[arg(long, help = "List scenes without entering the TUI")]
    list: bool,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    if args.list {
        for (name, description) in scenes::SCENES {
            println!("{name:16} {description}");
        }
        return Ok(());
    }
    if !scenes::SCENES.iter().any(|(name, _)| *name == args.scene) {
        return Err(format!("unknown scene '{}'; use --list to see available scenes", args.scene).into());
    }
    if !io::stderr().is_terminal() {
        return Err("run the layout example in an interactive terminal".into());
    }
    let (width, height) = crossterm::terminal::size()?;
    if width < 60
        || height < 12
        || std::env::var_os("NO_COLOR").is_some_and(|value| !value.is_empty())
        || std::env::var_os("TERM").is_some_and(|value| value == "dumb")
    {
        return Err("use a color terminal of at least 60x12 to run the layout example".into());
    }
    let progress = progress::Progress::stderr()?;
    scenes::populate(&progress, &args.scene);
    while !process::cancelled() {
        thread::sleep(Duration::from_millis(50));
    }
    // 退出预览仅恢复终端，不把未完成的模拟任务打印为失败摘要。
    drop(progress);
    Ok(())
}
