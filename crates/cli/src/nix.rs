//! 与 Nix 的全部交互。工具唯一一条取哈希的代码路径。

use std::process::Command;

/// 注入配置的固定占位哈希。必须固定：变动会改变中间 FOD 的 drvPath（ADR-0014）。
pub const FAKE: &str = "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";

#[derive(Debug)]
pub enum Error {
    /// 构建在到达哈希比对之前失败（缺 lockfile、404 等）。原样保留输出，不猜测
    /// （ADR-0003、ADR-0012）。
    NoGotLine(String),
    Nix(String),
}

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
        }
        self
    }

    fn label(&self) -> &str {
        match self {
            Self::Text(text) => text,
            Self::Counter { label, .. } => label,
            Self::Status { .. } => "Working",
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
    let out = crate::progress::command_output(&mut command).map_err(|error| Error::Nix(error.to_string()))?;
    if !out.status.success() {
        return Err(Error::Nix(String::from_utf8_lossy(&out.stderr).into_owned()));
    }
    serde_json::from_slice(&out.stdout).map_err(|error| Error::Nix(error.to_string()))
}

#[derive(Debug)]
enum Activity {
    Copy(Vec<String>),
    Download {
        done: u64,
        total: Option<u64>,
    },
    Build {
        name: Option<String>,
        phase: Option<(u64, String)>,
        download: Option<(u64, NixProgress)>,
    },
    Query(Vec<String>),
}

#[derive(Default)]
struct NixLog {
    activities: std::collections::BTreeMap<u64, Activity>,
    phase_sequence: u64,
    completed_download_bytes: u64,
    completed_download_total: u64,
    completed_download_total_unknown: bool,
    diagnostic: Option<String>,
}

impl NixLog {
    fn push(&mut self, line: &str) -> Option<String> {
        let Some(json) = line.strip_prefix("@nix ") else {
            return Some(line.into());
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
            return Some(line.into());
        };
        let Some(action) = value.get("action").and_then(serde_json::Value::as_str) else {
            return Some(line.into());
        };
        if action == "msg" {
            return value
                .get("msg")
                .or_else(|| value.get("raw_msg"))
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
                .or_else(|| Some(line.into()));
        }
        if !matches!(action, "start" | "stop" | "result") {
            return None;
        }
        let Some(id) = value.get("id").and_then(serde_json::Value::as_u64) else {
            return Some(line.into());
        };
        match action {
            "start" => {
                let activity = match value.get("type").and_then(serde_json::Value::as_u64) {
                    Some(100) => Activity::Copy(activity_fields(&value)),
                    Some(101) => Activity::Download { done: 0, total: None },
                    Some(105) => Activity::Build {
                        name: value
                            .get("fields")
                            .and_then(serde_json::Value::as_array)
                            .and_then(|fields| fields.first())
                            .and_then(serde_json::Value::as_str)
                            .map(str::to_owned),
                        phase: None,
                        download: None,
                    },
                    Some(108) => Activity::Query(activity_fields(&value)),
                    Some(_) => return None,
                    None => return Some(line.into()),
                };
                self.activities.insert(id, activity);
            }
            "stop" => {
                if let Some(Activity::Download { done, total }) = self.activities.remove(&id) {
                    self.completed_download_bytes = self.completed_download_bytes.saturating_add(done);
                    if let Some(total) = total {
                        self.completed_download_total = self.completed_download_total.saturating_add(total);
                    } else {
                        self.completed_download_total_unknown = true;
                    }
                }
            }
            "result" => {
                let Some(kind) = value.get("type").and_then(serde_json::Value::as_u64) else {
                    return Some(line.into());
                };
                let fields = value.get("fields").and_then(serde_json::Value::as_array);
                match (kind, self.activities.get_mut(&id), fields) {
                    (105, Some(Activity::Download { done, total }), Some(fields)) => {
                        *done = fields.first().and_then(serde_json::Value::as_u64).unwrap_or(*done);
                        *total = fields
                            .get(1)
                            .and_then(serde_json::Value::as_u64)
                            .filter(|value| *value > 0);
                    }
                    (104, Some(Activity::Build { phase, .. }), Some(fields)) => {
                        let Some(value) = fields.first().and_then(serde_json::Value::as_str) else {
                            return Some(line.into());
                        };
                        self.phase_sequence += 1;
                        *phase = Some((self.phase_sequence, value.into()));
                    }
                    (101, activity, Some(fields)) => {
                        let Some(value) = fields.first().and_then(serde_json::Value::as_str) else {
                            return Some(line.into());
                        };
                        if let Some(Activity::Build { download, .. }) = activity {
                            self.phase_sequence += 1;
                            *download = builder_progress(value).map(|progress| (self.phase_sequence, progress));
                        }
                        return Some(value.into());
                    }
                    (101 | 104 | 105, _, None) => return Some(line.into()),
                    _ => return None,
                }
            }
            _ => unreachable!(),
        }
        None
    }

    fn detail(&self) -> Option<NixProgress> {
        let downloads: Vec<_> = self
            .activities
            .values()
            .filter_map(|activity| match activity {
                Activity::Download { done, total } => Some((*done, *total)),
                _ => None,
            })
            .collect();
        if !downloads.is_empty() {
            let done = downloads.iter().fold(self.completed_download_bytes, |sum, (done, _)| {
                sum.saturating_add(*done)
            });
            let transfers = downloads.len();
            let total = (!self.completed_download_total_unknown && downloads.iter().all(|(_, total)| total.is_some()))
                .then(|| {
                    downloads.iter().fold(self.completed_download_total, |sum, (_, total)| {
                        sum.saturating_add(total.unwrap_or_default())
                    })
                });
            return Some(NixProgress::Counter {
                current: done,
                total,
                unit: NixProgressUnit::Bytes,
                label: format!(
                    "Downloading · {transfers} {}",
                    if transfers == 1 { "transfer" } else { "transfers" }
                ),
            });
        }
        if self
            .activities
            .values()
            .any(|activity| matches!(activity, Activity::Copy(_)))
        {
            return Some(NixProgress::Text("Copying Store Path".into()));
        }
        let builder_downloads: Vec<_> = self
            .activities
            .values()
            .filter_map(|activity| match activity {
                Activity::Build { download, .. } => download.as_ref(),
                _ => None,
            })
            .collect();
        if let Some((_, latest)) = builder_downloads.iter().max_by_key(|(sequence, _)| *sequence).copied() {
            if let NixProgress::Counter { unit, label, .. } = latest {
                let counters: Vec<_> = builder_downloads
                    .iter()
                    .filter_map(|(_, progress)| match progress {
                        NixProgress::Counter {
                            current,
                            total,
                            unit: candidate_unit,
                            label: candidate_label,
                        } if candidate_unit == unit && candidate_label == label => Some((*current, *total)),
                        _ => None,
                    })
                    .collect();
                let current = counters
                    .iter()
                    .fold(0_u64, |sum, (current, _)| sum.saturating_add(*current));
                let total = counters.iter().all(|(_, total)| total.is_some()).then(|| {
                    counters
                        .iter()
                        .fold(0_u64, |sum, (_, total)| sum.saturating_add(total.unwrap_or_default()))
                });
                return Some(NixProgress::Counter {
                    current,
                    total,
                    unit: *unit,
                    label: if counters.len() == 1 {
                        label.clone()
                    } else {
                        format!("{label} · {} transfers", counters.len())
                    },
                });
            }
            return Some(latest.clone());
        }
        if let Some((_, phase)) = self
            .activities
            .values()
            .filter_map(|activity| match activity {
                Activity::Build { phase, .. } => phase.as_ref(),
                _ => None,
            })
            .max_by_key(|(sequence, _)| *sequence)
        {
            return Some(NixProgress::Text(phase.clone()));
        }
        if self
            .activities
            .values()
            .any(|activity| matches!(activity, Activity::Build { .. }))
        {
            return Some(NixProgress::Text("Building".into()));
        }
        self.activities
            .values()
            .any(|activity| matches!(activity, Activity::Query(_)))
            .then(|| NixProgress::Text("Querying Cache".into()))
    }

    fn progress(&self) -> Option<NixProgress> {
        let detail = self.detail();
        let activities = self.activity_lines();
        if activities.is_empty() {
            detail
        } else {
            Some(NixProgress::Status {
                detail: detail.map(Box::new),
                activities,
            })
        }
    }

    fn activity_lines(&self) -> Vec<String> {
        let mut lines: Vec<_> = self
            .activities
            .values()
            .filter_map(|activity| match activity {
                Activity::Build { name, phase, download } => {
                    let mut line = name
                        .as_deref()
                        .map_or_else(|| "Building".into(), |name| format!("Building {name}"));
                    if let Some((_, phase)) = phase {
                        line.push_str(" · ");
                        line.push_str(phase);
                    } else if let Some((_, progress)) = download {
                        line.push_str(" · ");
                        line.push_str(progress.label());
                    }
                    Some(line)
                }
                Activity::Copy(fields) => Some(activity_line("Copying", fields)),
                Activity::Query(fields) => Some(activity_line("Querying", fields)),
                Activity::Download { .. } => None,
            })
            .collect();
        lines.extend(self.diagnostic.iter().cloned());
        lines
    }
}

fn activity_fields(value: &serde_json::Value) -> Vec<String> {
    value
        .get("fields")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_str)
        .map(str::to_owned)
        .collect()
}

fn activity_line(action: &str, fields: &[String]) -> String {
    if fields.is_empty() {
        action.into()
    } else {
        format!("{action} {}", fields.join(" → "))
    }
}

fn builder_progress(line: &str) -> Option<NixProgress> {
    curl_progress(line).or_else(|| git_progress(line))
}

fn curl_progress(line: &str) -> Option<NixProgress> {
    let fields: Vec<_> = line.split_whitespace().collect();
    if fields.len() < 9 {
        return None;
    }
    let percent: u8 = fields[0].parse().ok()?;
    let size = |value: &str| {
        value.chars().any(|character| character.is_ascii_digit())
            && value
                .chars()
                .all(|character| character.is_ascii_digit() || ".kMGT".contains(character))
    };
    if percent > 100
        || !size(fields[1])
        || !size(fields[3])
        || !(fields[8..].iter().any(|value| value.contains(':')) || fields.iter().all(|value| *value == "0"))
    {
        return None;
    }
    Some(NixProgress::Counter {
        current: parse_size(fields[3])?,
        total: (fields[1] != "0").then(|| parse_size(fields[1])).flatten(),
        unit: NixProgressUnit::Bytes,
        label: "Downloading".into(),
    })
}

fn git_progress(line: &str) -> Option<NixProgress> {
    if !line.contains('|') && !line.contains("Receiving objects:") && !line.contains("Downloading LFS objects:") {
        return None;
    }
    let marker = line.find("% (")?;
    let percent = line[..marker]
        .rsplit(|character: char| !character.is_ascii_digit())
        .next()?
        .parse::<u8>()
        .ok()?;
    let counts = line[marker + 3..].split_once(')')?.0;
    let (done, total) = counts.split_once('/')?;
    if percent > 100
        || done.is_empty()
        || total.is_empty()
        || !done.chars().all(|character| character.is_ascii_digit())
        || !total.chars().all(|character| character.is_ascii_digit())
    {
        return None;
    }
    Some(NixProgress::Counter {
        current: done.parse().ok()?,
        total: Some(total.parse().ok()?),
        unit: NixProgressUnit::Objects,
        label: if line.contains("Downloading LFS objects:") {
            "LFS objects"
        } else {
            "Git objects"
        }
        .into(),
    })
}

fn parse_size(value: &str) -> Option<u64> {
    let (number, multiplier) = match value.chars().last()? {
        'k' => (&value[..value.len() - 1], 1024_f64),
        'M' => (&value[..value.len() - 1], 1024_f64.powi(2)),
        'G' => (&value[..value.len() - 1], 1024_f64.powi(3)),
        'T' => (&value[..value.len() - 1], 1024_f64.powi(4)),
        _ => (value, 1.0),
    };
    Some((number.parse::<f64>().ok()? * multiplier).round() as u64)
}

fn build(
    drv_path: &str,
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
    crate::progress::configure_child_process(&mut command);
    let mut child = command.spawn().map_err(|error| Error::Nix(error.to_string()))?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| Error::Nix("无法读取 Nix stdout".into()))?;
    let stdout = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout.read_to_end(&mut bytes).map(|_| bytes)
    });
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| Error::Nix("无法读取 Nix stderr".into()))?;
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
    let mut stderr_error = None;
    let mut was_cancelled = false;
    loop {
        if crate::progress::cancelled() {
            was_cancelled = true;
            crate::progress::kill_child_tree(&mut child);
            break;
        }
        match receiver.recv_timeout(Duration::from_millis(50)) {
            Ok(Ok(line)) => {
                if let Some(line) = state.push(&line) {
                    diagnostics.push_str(&line);
                    diagnostics.push('\n');
                    state.diagnostic = Some(line);
                }
                let detail = state.progress();
                if detail != last_detail {
                    progress(detail.clone());
                    last_detail = detail;
                }
                state.diagnostic = None;
            }
            Ok(Err(error)) => {
                stderr_error = Some(error);
                crate::progress::kill_child_tree(&mut child);
                break;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if child
                    .try_wait()
                    .map_err(|error| Error::Nix(error.to_string()))?
                    .is_some()
                {
                    break;
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    let wait_result = child.wait();
    let _ = stderr.join();
    let stdout = stdout.join();
    progress(None);

    if was_cancelled {
        return Err(Error::Nix("已取消".into()));
    }
    if let Some(error) = stderr_error {
        return Err(Error::Nix(error.to_string()));
    }
    let status = wait_result.map_err(|error| Error::Nix(error.to_string()))?;
    let stdout = stdout
        .map_err(|_| Error::Nix("读取 Nix stdout 的线程异常退出".into()))?
        .map_err(|error| Error::Nix(error.to_string()))?;
    if !stdout.is_empty() {
        if !diagnostics.is_empty() && !diagnostics.ends_with('\n') {
            diagnostics.push('\n');
        }
        diagnostics.push_str(&String::from_utf8_lossy(&stdout));
    }

    // 退出码不可作为判据：404 的 fetchurl 曾以 0 退出且无 got 行（ADR-0003）。
    Ok((status, diagnostics))
}

pub fn resolve_hash(drv_path: &str, progress: impl FnMut(Option<NixProgress>)) -> Result<String, Error> {
    let (_, diagnostics) = build(drv_path, progress)?;
    // 退出码不可作为判据：404 的 fetchurl 曾以 0 退出且无 got 行（ADR-0003）。
    parse_got(&diagnostics).ok_or(Error::NoGotLine(diagnostics))
}

pub fn realize(drv_path: &str, progress: impl FnMut(Option<NixProgress>)) -> Result<(), Error> {
    let (status, diagnostics) = build(drv_path, progress)?;
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
    use super::{parse_got, NixLog, NixProgress, NixProgressUnit};

    /// 实测样本，取自本机 Nix 2.35.2（ADR-0003）。
    const MISMATCH: &str = concat!(
        "error: hash mismatch in fixed-output derivation: \n",
        "         specified: sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=\n",
        "            got:    sha256-BlpIDik4hkU4c+KCyAmgUURIN362RDQID/qo6Ojp2Ek=\n",
    );

    /// npmDeps 缺 lockfile 时的实测输出：无 got 行（ADR-0012）。
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
            r#"@nix {"action":"result","id":1,"type":101,"fields":["展开对象中: 42% (13/31), 4.2 MiB | 1.0 MiB/s"]}"#,
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

        log.push(r#"@nix {"action":"start","id":1,"type":108,"fields":["cache"]}"#);
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
        log.push(r#"@nix {"action":"start","id":2,"type":108,"fields":["https://cache.example/path?token=hidden"]}"#);
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
}
