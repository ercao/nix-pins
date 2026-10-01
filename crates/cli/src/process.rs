//! 统一管理取消信号、子进程组和输出回收，执行层无需依赖终端展示。

use std::io;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

pub(crate) fn install_ctrlc_handler() -> Result<(), ctrlc::Error> {
    CANCELLED.store(false, Ordering::SeqCst);
    ctrlc::set_handler(cancel)
}

static CANCELLED: AtomicBool = AtomicBool::new(false);

#[cfg(test)]
pub(crate) fn reset_cancelled() {
    CANCELLED.store(false, Ordering::SeqCst);
}

pub fn cancelled() -> bool {
    CANCELLED.load(Ordering::SeqCst)
}

pub(crate) fn command_output(command: &mut Command) -> io::Result<Output> {
    command_output_with_cancel(command, &CANCELLED)
}

/// 同时排空 stdout/stderr，轮询取消信号并等待子进程回收，避免管道填满后相互等待。
fn command_output_with_cancel(command: &mut Command, cancelled: &AtomicBool) -> io::Result<Output> {
    use std::io::Read;
    use std::time::Duration;

    configure_child_process(command);
    let mut child = command.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("cannot read child stdout"))?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| io::Error::other("cannot read child stderr"))?;
    let stdout = thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout.read_to_end(&mut bytes).map(|_| bytes)
    });
    let stderr = thread::spawn(move || {
        let mut bytes = Vec::new();
        stderr.read_to_end(&mut bytes).map(|_| bytes)
    });
    let status = loop {
        if cancelled.load(Ordering::SeqCst) {
            kill_child_tree(&mut child);
            break child.wait()?;
        }
        if let Some(status) = child.try_wait()? {
            break status;
        }
        thread::sleep(Duration::from_millis(25));
    };
    let stdout = stdout
        .join()
        .map_err(|_| io::Error::other("child stdout reader thread panicked"))??;
    let stderr = stderr
        .join()
        .map_err(|_| io::Error::other("child stderr reader thread panicked"))??;
    Ok(Output { status, stdout, stderr })
}

/// Unix 子进程使用独立进程组，取消时才能一并终止 Shell 派生出的工作进程。
pub(crate) fn configure_child_process(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    command.process_group(0);
}

pub(crate) fn kill_child_tree(child: &mut std::process::Child) {
    // SAFETY: 子进程在 spawn 前已进入以自身 pid 为 id 的独立进程组。
    if unsafe { libc::kill(-(child.id() as i32), libc::SIGKILL) } == -1 {
        let _ = child.kill();
    }
}

pub(crate) fn cancel() {
    CANCELLED.store(true, Ordering::SeqCst);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancellation_stops_a_running_child() {
        let cancelled = AtomicBool::new(true);
        let mut command = Command::new("sh");
        command.args(["-c", "sleep 5"]);

        let started = std::time::Instant::now();
        let output = command_output_with_cancel(&mut command, &cancelled).unwrap();

        assert!(!output.status.success());
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
    }
}
