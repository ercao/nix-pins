//! 与 Nix 的全部交互。工具唯一一条取哈希的代码路径。

mod log;

use log::*;
pub(crate) use log::{activity_message_level, activity_summary, sanitize_activity, terminal_text};
use prodash::messages::MessageLevel;
use std::process::Command;

/// 注入配置的固定占位哈希。必须固定：变动会改变中间 FOD 的 drvPath（ADR-0003）。
pub const FAKE: &str = "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";

#[derive(Debug)]
pub enum Error {
    /// 构建在到达哈希比对之前失败（缺 lockfile、404 等）。原样保留输出，不猜测
    /// （ADR-0003）。
    NoGotLine(String),
    Nix(String),
}

/// Nix 日志的显示事件；未知总量保留为 None，诊断消息与阶段进度分别处理。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NixProgress {
    Text(String),
    Counter {
        current: u64,
        total: Option<u64>,
        unit: NixProgressUnit,
        label: String,
    },
    Status {
        detail: Option<Box<NixProgress>>,
        activities: Vec<String>,
    },
    Message {
        level: MessageLevel,
        text: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NixProgressUnit {
    Bytes,
    Objects,
}

impl NixProgress {
    pub fn with_prefix(mut self, prefix: &str) -> Self {
        match &mut self {
            Self::Text(text) => *text = format!("{prefix} · {text}"),
            Self::Counter { label, .. } => *label = format!("{prefix} · {label}"),
            Self::Status { detail, .. } => {
                let value = detail
                    .take()
                    .map_or_else(|| Self::Text(prefix.into()), |value| (*value).with_prefix(prefix));
                *detail = Some(Box::new(value));
            }
            Self::Message { .. } => {}
        }
        self
    }

    fn label(&self) -> &str {
        match self {
            Self::Text(text) => text,
            Self::Counter { label, .. } => label,
            Self::Status { .. } | Self::Message { .. } => "Working",
        }
    }
}

impl From<String> for NixProgress {
    fn from(value: String) -> Self {
        Self::Text(value)
    }
}

impl From<&str> for NixProgress {
    fn from(value: &str) -> Self {
        Self::Text(value.into())
    }
}

/// 求值一个 Nix 表达式并以 JSON 返回。
pub fn eval_json(expr: &str) -> Result<serde_json::Value, Error> {
    let mut command = Command::new("nix");
    command.args(["eval", "--impure", "--json", "--expr", expr]);
    let out = crate::process::command_output(&mut command).map_err(|error| Error::Nix(error.to_string()))?;
    if !out.status.success() {
        return Err(Error::Nix(String::from_utf8_lossy(&out.stderr).into_owned()));
    }
    serde_json::from_slice(&out.stdout).map_err(|error| Error::Nix(error.to_string()))
}

/// 哈希探测先暂存失败消息，等完整输出确认是否仅为预期的 hash mismatch。
fn emit_diagnostic(
    message: (MessageLevel, String),
    expected_hash: bool,
    deferred: &mut Vec<(MessageLevel, String)>,
    progress: &mut impl FnMut(Option<NixProgress>),
) {
    if expected_hash && message.0 == MessageLevel::Failure {
        deferred.push(message);
    } else {
        progress(Some(diagnostic_summary(message.0, &message.1)));
    }
}

fn build(
    drv_path: &str,
    expected_hash: bool,
    mut progress: impl FnMut(Option<NixProgress>),
) -> Result<(std::process::ExitStatus, String), Error> {
    use std::io::{BufRead, BufReader, Read};
    use std::process::Stdio;
    use std::sync::mpsc;
    use std::time::Duration;

    let mut command = Command::new("nix");
    command
        .args([
            "build",
            "--no-link",
            "-L",
            "--log-format",
            "internal-json",
            &format!("{drv_path}^out"),
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    crate::process::configure_child_process(&mut command);
    let mut child = command.spawn().map_err(|error| Error::Nix(error.to_string()))?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| Error::Nix("cannot read Nix stdout".into()))?;
    let stdout = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout.read_to_end(&mut bytes).map(|_| bytes)
    });
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| Error::Nix("cannot read Nix stderr".into()))?;
    let (lines, receiver) = mpsc::channel();
    let stderr = std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines() {
            if lines.send(line).is_err() {
                break;
            }
        }
    });
    let mut state = NixLog::default();
    let mut last_detail = None;
    let mut diagnostics = String::new();
    let mut pending_message: Option<(MessageLevel, String)> = None;
    let mut deferred = Vec::new();
    let mut stderr_error = None;
    let mut was_cancelled = false;
    loop {
        if crate::process::cancelled() {
            was_cancelled = true;
            crate::process::kill_child_tree(&mut child);
            break;
        }
        match receiver.recv_timeout(Duration::from_millis(50)) {
            Ok(Ok(line)) => {
                if let Some(line) = state.push(&line) {
                    diagnostics.push_str(&line);
                    diagnostics.push('\n');
                    if !state.diagnostic_is_progress {
                        let text = terminal_text(&line);
                        let trimmed = text.trim_start();
                        let continuation = state.diagnostic_level.is_none()
                            && (text.starts_with(char::is_whitespace)
                                || trimmed.starts_with("specified:")
                                || trimmed.starts_with("got:"));
                        if let Some((_, pending)) = pending_message.as_mut().filter(|_| continuation) {
                            pending.push('\n');
                            pending.push_str(&line);
                        } else {
                            if let Some(message) = pending_message.take() {
                                emit_diagnostic(message, expected_hash, &mut deferred, &mut progress);
                            }
                            let level = match state.diagnostic_level {
                                Some(0) => MessageLevel::Failure,
                                Some(_) => MessageLevel::Info,
                                None => activity_message_level(&text),
                            };
                            let text = if state.diagnostic_level == Some(1) && !trimmed.starts_with("warning:") {
                                format!("warning: {line}")
                            } else {
                                line
                            };
                            pending_message = Some((level, text));
                        }
                    }
                } else if let Some(message) = pending_message.take() {
                    emit_diagnostic(message, expected_hash, &mut deferred, &mut progress);
                }
                let detail = state.progress();
                if detail != last_detail {
                    progress(detail.clone());
                    last_detail = detail;
                }
            }
            Ok(Err(error)) => {
                stderr_error = Some(error);
                crate::process::kill_child_tree(&mut child);
                break;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    let wait_result = child.wait();
    let _ = stderr.join();
    let stdout = stdout.join();

    if was_cancelled {
        return Err(Error::Nix("Cancelled".into()));
    }
    if let Some(error) = stderr_error {
        return Err(Error::Nix(error.to_string()));
    }
    let status = wait_result.map_err(|error| Error::Nix(error.to_string()))?;
    let stdout = stdout
        .map_err(|_| Error::Nix("Nix stdout reader thread panicked".into()))?
        .map_err(|error| Error::Nix(error.to_string()))?;
    if !stdout.is_empty() {
        if !diagnostics.is_empty() && !diagnostics.ends_with('\n') {
            diagnostics.push('\n');
        }
        diagnostics.push_str(&String::from_utf8_lossy(&stdout));
    }

    if let Some(message) = pending_message {
        emit_diagnostic(message, expected_hash, &mut deferred, &mut progress);
    }
    finish_diagnostics(expected_hash, &diagnostics, deferred, &mut progress);
    progress(None);

    // 退出码不可作为判据：404 的 fetchurl 曾以 0 退出且无 got 行（ADR-0003）。
    Ok((status, diagnostics))
}

/// 找到 got 行后隐藏预期的哈希不匹配诊断；真正的构建错误仍进入消息流。
fn finish_diagnostics(
    expected_hash: bool,
    diagnostics: &str,
    deferred: Vec<(MessageLevel, String)>,
    progress: &mut impl FnMut(Option<NixProgress>),
) {
    let hash = expected_hash.then(|| parse_got(diagnostics)).flatten();
    for (level, text) in deferred {
        let clean = terminal_text(&text);
        let expected = clean.contains("hash mismatch") || parse_got(&clean).is_some() || clean.trim() == "❌";
        if hash.is_none() || !expected {
            progress(Some(diagnostic_summary(level, &text)));
        }
    }
    if let Some(hash) = hash {
        progress(Some(NixProgress::Message {
            level: MessageLevel::Success,
            text: format!("Resolved hash {hash}"),
        }));
    }
}

/// 占位哈希构建预期失败，以 got 行作为成功反馈而非依赖进程退出码。
pub fn resolve_hash(drv_path: &str, progress: impl FnMut(Option<NixProgress>)) -> Result<String, Error> {
    let (_, diagnostics) = build(drv_path, true, progress)?;
    // 退出码不可作为判据：404 的 fetchurl 曾以 0 退出且无 got 行（ADR-0003）。
    parse_got(&diagnostics).ok_or(Error::NoGotLine(diagnostics))
}

/// 实现非哈希探测的 derivation（如补丁源码），此时必须检查构建退出码。
pub fn realize(drv_path: &str, progress: impl FnMut(Option<NixProgress>)) -> Result<(), Error> {
    let (status, diagnostics) = build(drv_path, false, progress)?;
    if status.success() {
        Ok(())
    } else {
        Err(Error::Nix(diagnostics))
    }
}

/// 从 hash mismatch 输出中提取 got 行的哈希。
fn parse_got(log: &str) -> Option<String> {
    log.lines().find_map(|line| {
        let (_, rest) = line.split_once("got:")?;
        let start = rest.find("sha256-").or_else(|| rest.find("sha512-"))?;
        let hash = rest[start..]
            .chars()
            .take_while(|character| character.is_ascii_alphanumeric() || "+/=-".contains(*character))
            .collect::<String>();
        (hash.len() > "sha256-".len()).then_some(hash)
    })
}

#[cfg(test)]
mod tests {
    use super::{NixLog, NixProgress, NixProgressUnit, parse_got};

    /// 实测样本，取自本机 Nix 2.35.2（ADR-0003）。
    const MISMATCH: &str = concat!(
        "error: hash mismatch in fixed-output derivation: \n",
        "         specified: sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=\n",
        "            got:    sha256-BlpIDik4hkU4c+KCyAmgUURIN362RDQID/qo6Ojp2Ek=\n",
    );

    /// npmDeps 缺 lockfile 时的实测输出：无 got 行（ADR-0003）。
    const NO_LOCKFILE: &str = concat!(
        "svgo> ERROR: No lock file!\n",
        "error: Cannot build npm-deps.drv\n",
        "       Reason: builder failed with exit code 1.\n",
    );

    #[test]
    fn extracts_got_hash() {
        assert_eq!(
            parse_got(MISMATCH).as_deref(),
            Some("sha256-BlpIDik4hkU4c+KCyAmgUURIN362RDQID/qo6Ojp2Ek=")
        );
    }

    #[test]
    fn no_got_line_is_none_not_garbage() {
        assert_eq!(parse_got(NO_LOCKFILE), None);
    }

    /// specified 行也含哈希，但排在 got 之前，不得被误取。
    #[test]
    fn does_not_return_specified() {
        assert_ne!(parse_got(MISMATCH).as_deref(), Some(super::FAKE));
    }

    #[test]
    fn extracts_colored_got_hash_after_an_invalid_got_line() {
        let log = concat!(
            "diagnostic got: not-a-hash\n",
            "            got:    \u{1b}[35;1m",
            "sha256-NWcqJkIPRKGSr9n6X2DWlS4/Kzsg+k4ue+mMuo/drn4=",
            "\u{1b}[0m\n",
        );

        assert_eq!(
            parse_got(log).as_deref(),
            Some("sha256-NWcqJkIPRKGSr9n6X2DWlS4/Kzsg+k4ue+mMuo/drn4=")
        );
    }

    #[test]
    fn internal_json_aggregates_downloads_without_exposing_urls() {
        let mut log = NixLog::default();

        assert_eq!(
            log.push(r#"@nix {"action":"start","id":1,"type":101,"fields":["https://secret.invalid/a"]}"#),
            None
        );
        assert_eq!(
            log.push(r#"@nix {"action":"start","id":2,"type":101,"fields":["https://secret.invalid/b"]}"#),
            None
        );
        log.push(r#"@nix {"action":"result","id":1,"type":105,"fields":[1048576,2097152,0,0]}"#);
        log.push(r#"@nix {"action":"result","id":2,"type":105,"fields":[524288,1048576,0,0]}"#);

        assert_eq!(
            log.detail(),
            Some(NixProgress::Counter {
                current: 1_572_864,
                total: Some(3_145_728),
                unit: NixProgressUnit::Bytes,
                label: "Downloading · 2 transfers".into(),
            })
        );
    }

    #[test]
    fn internal_json_keeps_completed_downloads_in_the_next_total() {
        let mut log = NixLog::default();
        log.push(r#"@nix {"action":"start","id":1,"type":101,"fields":["hidden"]}"#);
        log.push(r#"@nix {"action":"result","id":1,"type":105,"fields":[1048576,1048576,0,0]}"#);
        log.push(r#"@nix {"action":"stop","id":1}"#);
        log.push(r#"@nix {"action":"start","id":2,"type":101,"fields":["hidden"]}"#);
        log.push(r#"@nix {"action":"result","id":2,"type":105,"fields":[524288,2097152,0,0]}"#);

        assert_eq!(
            log.detail(),
            Some(NixProgress::Counter {
                current: 1_572_864,
                total: Some(3_145_728),
                unit: NixProgressUnit::Bytes,
                label: "Downloading · 1 transfer".into(),
            })
        );
    }

    #[test]
    fn internal_json_omits_percentage_when_any_download_total_is_unknown() {
        let mut log = NixLog::default();
        log.push(r#"@nix {"action":"start","id":1,"type":101,"fields":["hidden"]}"#);
        log.push(r#"@nix {"action":"result","id":1,"type":105,"fields":[1048576,0,0,0]}"#);

        assert_eq!(
            log.detail(),
            Some(NixProgress::Counter {
                current: 1_048_576,
                total: None,
                unit: NixProgressUnit::Bytes,
                label: "Downloading · 1 transfer".into(),
            })
        );
    }

    #[test]
    fn internal_json_shows_completed_download_without_percentage() {
        let mut log = NixLog::default();
        log.push(r#"@nix {"action":"start","id":1,"type":101,"fields":["hidden"]}"#);
        log.push(r#"@nix {"action":"result","id":1,"type":105,"fields":[1048576,1048576,0,0]}"#);
        assert_eq!(
            log.detail(),
            Some(NixProgress::Counter {
                current: 1_048_576,
                total: Some(1_048_576),
                unit: NixProgressUnit::Bytes,
                label: "Downloading · 1 transfer".into(),
            })
        );
    }

    #[test]
    fn completed_unknown_download_keeps_later_total_unknown() {
        let mut log = NixLog::default();
        log.push(r#"@nix {"action":"start","id":1,"type":101,"fields":["hidden"]}"#);
        log.push(r#"@nix {"action":"result","id":1,"type":105,"fields":[10,0,0,0]}"#);
        log.push(r#"@nix {"action":"stop","id":1,"type":101}"#);
        log.push(r#"@nix {"action":"start","id":2,"type":101,"fields":["hidden"]}"#);
        log.push(r#"@nix {"action":"result","id":2,"type":105,"fields":[5,20,0,0]}"#);

        assert_eq!(
            log.detail(),
            Some(NixProgress::Counter {
                current: 15,
                total: None,
                unit: NixProgressUnit::Bytes,
                label: "Downloading · 1 transfer".into(),
            })
        );
    }

    #[test]
    fn internal_json_reads_fetchurl_curl_progress() {
        let mut log = NixLog::default();
        log.push(r#"@nix {"action":"start","id":1,"type":105,"fields":["hidden.drv"]}"#);

        log.push(r#"@nix {"action":"result","id":1,"type":101,"fields":[" 0 0 0 0 0 0 0 0 0"]}"#);
        assert_eq!(
            log.detail(),
            Some(NixProgress::Counter {
                current: 0,
                total: None,
                unit: NixProgressUnit::Bytes,
                label: "Downloading".into(),
            })
        );
        log.push(
            r#"@nix {"action":"result","id":1,"type":101,"fields":[" 42 10.0M 0 4.2M 0 0 1.0M 0 00:10 00:04 00:06"]}"#,
        );
        assert_eq!(
            log.detail(),
            Some(NixProgress::Counter {
                current: 4_404_019,
                total: Some(10_485_760),
                unit: NixProgressUnit::Bytes,
                label: "Downloading".into(),
            })
        );
        log.push(r#"@nix {"action":"result","id":1,"type":101,"fields":["unpacking source archive"]}"#);
        assert_eq!(log.detail(), Some(NixProgress::Text("Building".into())));
    }

    #[test]
    fn internal_json_reads_fetchgit_and_lfs_progress() {
        let mut log = NixLog::default();
        log.push(r#"@nix {"action":"start","id":1,"type":105,"fields":["hidden.drv"]}"#);

        log.push(
            r#"@nix {"action":"result","id":1,"type":101,"fields":["\u5c55\u5f00\u5bf9\u8c61\u4e2d: 42% (13/31), 4.2 MiB | 1.0 MiB/s"]}"#,
        );
        assert_eq!(
            log.detail(),
            Some(NixProgress::Counter {
                current: 13,
                total: Some(31),
                unit: NixProgressUnit::Objects,
                label: "Git objects".into(),
            })
        );
        log.push(
            r#"@nix {"action":"result","id":1,"type":101,"fields":["Downloading LFS objects: 75% (3/4), 12 MB | 2 MB/s"]}"#,
        );
        assert_eq!(
            log.detail(),
            Some(NixProgress::Counter {
                current: 3,
                total: Some(4),
                unit: NixProgressUnit::Objects,
                label: "LFS objects".into(),
            })
        );
    }

    #[test]
    fn internal_json_aggregates_parallel_builder_transfers() {
        let mut log = NixLog::default();
        for id in [1, 2] {
            log.push(&format!(
                r#"@nix {{"action":"start","id":{id},"type":105,"fields":["hidden.drv"]}}"#
            ));
        }
        log.push(
            r#"@nix {"action":"result","id":1,"type":101,"fields":[" 25 8.0M 0 2.0M 0 0 1.0M 0 00:08 00:02 00:06"]}"#,
        );
        log.push(
            r#"@nix {"action":"result","id":2,"type":101,"fields":[" 50 4.0M 0 2.0M 0 0 1.0M 0 00:04 00:02 00:02"]}"#,
        );

        assert_eq!(
            log.detail(),
            Some(NixProgress::Counter {
                current: 4_194_304,
                total: Some(12_582_912),
                unit: NixProgressUnit::Bytes,
                label: "Downloading · 2 transfers".into(),
            })
        );

        log.push(r#"@nix {"action":"result","id":2,"type":101,"fields":[" 0 0 0 3.0M 0 0 1.0M 0 00:04 00:03 00:01"]}"#);
        assert_eq!(
            log.detail(),
            Some(NixProgress::Counter {
                current: 5_242_880,
                total: None,
                unit: NixProgressUnit::Bytes,
                label: "Downloading · 2 transfers".into(),
            })
        );
    }

    #[test]
    fn internal_json_uses_fixed_detail_priority_and_preserves_diagnostics() {
        let mut log = NixLog::default();

        log.push(r#"@nix {"action":"start","id":1,"type":109,"fields":["cache"]}"#);
        assert_eq!(log.detail(), Some(NixProgress::Text("Querying Cache".into())));
        log.push(r#"@nix {"action":"start","id":2,"type":105,"fields":["/nix/store/demo.drv"]}"#);
        assert_eq!(log.detail(), Some(NixProgress::Text("Building".into())));
        log.push(r#"@nix {"action":"result","id":2,"type":104,"fields":["buildPhase"]}"#);
        assert_eq!(log.detail(), Some(NixProgress::Text("buildPhase".into())));
        log.push(r#"@nix {"action":"start","id":5,"type":105,"fields":["/nix/store/newer.drv"]}"#);
        log.push(r#"@nix {"action":"result","id":5,"type":104,"fields":["installPhase"]}"#);
        assert_eq!(log.detail(), Some(NixProgress::Text("installPhase".into())));
        log.push(r#"@nix {"action":"stop","id":5}"#);
        assert_eq!(log.detail(), Some(NixProgress::Text("buildPhase".into())));
        log.push(r#"@nix {"action":"start","id":3,"type":100,"fields":["from","to"]}"#);
        assert_eq!(log.detail(), Some(NixProgress::Text("Copying Store Path".into())));
        log.push(r#"@nix {"action":"start","id":4,"type":101,"fields":["https://secret.invalid"]}"#);
        assert!(matches!(
            log.detail(),
            Some(NixProgress::Counter {
                unit: NixProgressUnit::Bytes,
                ..
            })
        ));

        assert_eq!(
            log.push(r#"@nix {"action":"msg","level":0,"msg":"got: sha256-real"}"#),
            Some("got: sha256-real".into())
        );
        assert_eq!(
            log.push(r#"@nix {"action":"start","id":9}"#),
            Some(r#"@nix {"action":"start","id":9}"#.into())
        );
        assert_eq!(log.push(r#"@nix {"action":"start","id":9,"type":999}"#), None);
        for frame in [
            r#"@nix {"action":"start","id":10,"type":0,"fields":[]}"#,
            r#"@nix {"action":"start","id":11,"type":102,"fields":[]}"#,
            r#"@nix {"action":"start","id":12,"type":103,"fields":[]}"#,
            r#"@nix {"action":"start","id":13,"type":104,"fields":[]}"#,
            r#"@nix {"action":"result","id":10,"type":105,"fields":[0,1,0,0]}"#,
            r#"@nix {"action":"result","id":10,"type":106,"fields":[101,0]}"#,
        ] {
            assert_eq!(log.push(frame), None);
        }
        assert_eq!(log.push("ordinary diagnostic"), Some("ordinary diagnostic".into()));
        assert_eq!(log.push("@nix {broken"), Some("@nix {broken".into()));
    }

    #[test]
    fn internal_json_exposes_parallel_build_activity_lines() {
        let mut log = NixLog::default();
        log.push(r#"@nix {"action":"start","id":1,"type":105,"fields":["/nix/store/aaa-demo.drv"]}"#);
        log.push(r#"@nix {"action":"result","id":1,"type":104,"fields":["buildPhase"]}"#);
        log.push(r#"@nix {"action":"start","id":2,"type":105,"fields":["/nix/store/bbb-other.drv"]}"#);
        log.push(r#"@nix {"action":"result","id":2,"type":104,"fields":["installPhase"]}"#);

        assert_eq!(
            log.progress(),
            Some(NixProgress::Status {
                detail: Some(Box::new(NixProgress::Text("installPhase".into()))),
                activities: vec![
                    "Building /nix/store/aaa-demo.drv · buildPhase".into(),
                    "Building /nix/store/bbb-other.drv · installPhase".into(),
                ],
            })
        );
    }

    #[test]
    fn internal_json_keeps_copy_query_and_build_paths_in_activity_lines() {
        let mut log = NixLog::default();
        log.push(r#"@nix {"action":"start","id":1,"type":100,"fields":["/nix/store/from","/nix/store/to"]}"#);
        log.push(r#"@nix {"action":"start","id":2,"type":109,"fields":["https://cache.example/path?token=hidden"]}"#);
        log.push(r#"@nix {"action":"start","id":3,"type":105,"fields":["/nix/store/demo.drv"]}"#);

        assert_eq!(
            log.activity_lines(),
            vec![
                "Copying /nix/store/from → /nix/store/to".to_owned(),
                "Querying https://cache.example/path?token=hidden".to_owned(),
                "Building /nix/store/demo.drv".to_owned(),
            ]
        );
    }

    #[test]
    fn substitutes_are_copy_activities_and_queries_use_type_109() {
        let mut log = NixLog::default();
        log.push(r#"@nix {"action":"start","id":1,"type":108,"fields":["/nix/store/source","https://cache.example"]}"#);
        log.push(r#"@nix {"action":"start","id":2,"type":109,"fields":["/nix/store/source","https://cache.example"]}"#);
        assert_eq!(
            log.activity_lines(),
            vec![
                "Copying /nix/store/source → https://cache.example".to_owned(),
                "Querying /nix/store/source → https://cache.example".to_owned()
            ]
        );
        log.push(r#"@nix {"action":"stop","id":1}"#);
        assert_eq!(log.detail(), Some(NixProgress::Text("Querying Cache".into())));
        log.push(r#"@nix {"action":"stop","id":2}"#);
        assert_eq!(log.progress(), None);
    }

    #[test]
    fn builder_counters_keep_diagnostics_but_do_not_enter_the_feed() {
        let mut log = NixLog::default();
        log.push(r#"@nix {"action":"start","id":1,"type":105,"fields":["/nix/store/demo.drv"]}"#);
        let text = "Receiving objects: 42% (13/31), 4 MiB | 1 MiB/s\r";
        let frame = serde_json::json!({"action":"result","id":1,"type":101,"fields":[text]});
        assert_eq!(log.push(&format!("@nix {frame}")), Some(text.into()));
        assert!(log.diagnostic_is_progress);
        assert!(matches!(
            log.detail(),
            Some(NixProgress::Counter {
                current: 13,
                total: Some(31),
                ..
            })
        ));
        log.push(r#"@nix {"action":"result","id":1,"type":101,"fields":["packages 42 10 0 4 0 0 1 0 total"]}"#);
        assert!(!log.diagnostic_is_progress);
    }

    #[test]
    fn diagnostic_blocks_keep_levels_positions_and_trace_order() {
        let mut log = NixLog::default();
        let text =
            "       … while evaluating\n       at config.nix:2:3\nerror: invalid declaration\n       source line";
        let frame = serde_json::json!({"action":"msg","level":0,"msg":text});
        assert_eq!(log.push(&format!("@nix {frame}")), Some(text.into()));
        assert_eq!(log.diagnostic_level, Some(0));
        assert_eq!(
            super::diagnostic_summary(super::MessageLevel::Failure, text),
            NixProgress::Message {
                level: super::MessageLevel::Failure,
                text: "error: invalid declaration".into()
            }
        );
        let frame = serde_json::json!({"action":"msg","level":1,"raw_msg":"cache warning",
            "file":"config.nix","line":2,"column":3,"trace":[
                {"raw_msg":"first frame","file":"a.nix","line":4,"column":5},
                {"raw_msg":"second frame","file":"b.nix","line":6,"column":7}]});
        assert_eq!(log.push(&format!("@nix {frame}")), Some(
            "cache warning\n       at config.nix:2:3\n       first frame\n       at a.nix:4:5\n       second frame\n       at b.nix:6:7".into()));
        assert_eq!(log.diagnostic_level, Some(1));
    }

    #[test]
    fn only_hash_resolution_can_reclassify_expected_mismatch_as_success() {
        let mut messages = Vec::new();
        let mut capture = |message: Option<NixProgress>| messages.push(message.unwrap());
        let mut deferred = Vec::new();
        super::emit_diagnostic(
            (super::MessageLevel::Failure, "❌ \u{1b}[31;1m\u{1b}[0m".into()),
            true,
            &mut deferred,
            &mut capture,
        );
        super::emit_diagnostic(
            (super::MessageLevel::Failure, MISMATCH.into()),
            true,
            &mut deferred,
            &mut capture,
        );
        assert_eq!(deferred.len(), 2);
        super::finish_diagnostics(true, MISMATCH, deferred, &mut capture);
        assert!(matches!(
            messages.as_slice(),
            [NixProgress::Message {
                level: super::MessageLevel::Success,
                ..
            }]
        ));
        messages.clear();
        let mut capture = |message: Option<NixProgress>| messages.push(message.unwrap());
        let mut deferred = Vec::new();
        super::emit_diagnostic(
            (super::MessageLevel::Failure, NO_LOCKFILE.into()),
            true,
            &mut deferred,
            &mut capture,
        );
        super::finish_diagnostics(true, NO_LOCKFILE, deferred, &mut capture);
        assert!(matches!(
            messages.as_slice(),
            [NixProgress::Message {
                level: super::MessageLevel::Failure,
                ..
            }]
        ));
        messages.clear();
        let mut capture = |message: Option<NixProgress>| messages.push(message.unwrap());
        let mut deferred = Vec::new();
        super::emit_diagnostic(
            (super::MessageLevel::Failure, MISMATCH.into()),
            false,
            &mut deferred,
            &mut capture,
        );
        super::finish_diagnostics(false, MISMATCH, deferred, &mut capture);
        assert!(matches!(
            messages.as_slice(),
            [NixProgress::Message {
                level: super::MessageLevel::Failure,
                ..
            }]
        ));
    }
}
