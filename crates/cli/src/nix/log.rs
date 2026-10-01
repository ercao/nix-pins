//! 解析 Nix 活动、计数和诊断文本；未知事件不决定执行结果。

use super::{NixProgress, NixProgressUnit};
use prodash::messages::MessageLevel;
use std::sync::OnceLock;

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

/// 跟踪单次 Nix 子进程内的活动 ID，活动结束后仍保留已完成下载的累计计数。
#[derive(Default)]
pub(super) struct NixLog {
    activities: std::collections::BTreeMap<u64, Activity>,
    phase_sequence: u64,
    completed_download_bytes: u64,
    completed_download_total: u64,
    completed_download_total_unknown: bool,
    pub(super) diagnostic_level: Option<u64>,
    pub(super) diagnostic_is_progress: bool,
}

impl NixLog {
    /// 更新已支持的活动事件；返回可保留的诊断文本，无法解码的输入仍按原文返回。
    pub(super) fn push(&mut self, line: &str) -> Option<String> {
        self.diagnostic_level = None;
        self.diagnostic_is_progress = false;
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
            self.diagnostic_level = value.get("level").and_then(serde_json::Value::as_u64);
            return diagnostic_text(&value).or_else(|| Some(line.into()));
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
                    Some(100 | 108) => Activity::Copy(activity_fields(&value)),
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
                    Some(109) => Activity::Query(activity_fields(&value)),
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
                            let text = terminal_text(value);
                            *download = builder_progress(&text).map(|progress| (self.phase_sequence, progress));
                            self.diagnostic_is_progress = download.is_some();
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

    /// 优先显示下载、复制、构建进度和阶段；只聚合单位与标签一致的构建计数。
    pub(super) fn detail(&self) -> Option<NixProgress> {
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

    pub(super) fn progress(&self) -> Option<NixProgress> {
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

    pub(super) fn activity_lines(&self) -> Vec<String> {
        let lines: Vec<_> = self
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
        lines
    }
}

pub(super) fn diagnostic_text(value: &serde_json::Value) -> Option<String> {
    if let Some(text) = value.get("msg").and_then(serde_json::Value::as_str) {
        return Some(text.into());
    }
    let mut text = value.get("raw_msg")?.as_str()?.to_owned();
    let location = |value: &serde_json::Value| {
        value.get("file").and_then(serde_json::Value::as_str).map(|file| {
            format!(
                "{file}:{}:{}",
                value.get("line").and_then(serde_json::Value::as_u64).unwrap_or(0),
                value.get("column").and_then(serde_json::Value::as_u64).unwrap_or(0)
            )
        })
    };
    if let Some(position) = location(value) {
        text.push_str(&format!("\n       at {position}"));
    }
    if let Some(trace) = value.get("trace").and_then(serde_json::Value::as_array) {
        for frame in trace {
            if let Some(hint) = frame.get("raw_msg").and_then(serde_json::Value::as_str) {
                text.push_str(&format!("\n       {hint}"));
            }
            if let Some(position) = location(frame) {
                text.push_str(&format!("\n       at {position}"));
            }
        }
    }
    Some(text)
}

pub(super) fn diagnostic_summary(level: MessageLevel, text: &str) -> NixProgress {
    NixProgress::Message {
        level,
        text: activity_summary(text),
    }
}

pub(super) fn activity_fields(value: &serde_json::Value) -> Vec<String> {
    value
        .get("fields")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_str)
        .map(str::to_owned)
        .collect()
}

pub(super) fn activity_line(action: &str, fields: &[String]) -> String {
    if fields.is_empty() {
        action.into()
    } else {
        format!("{action} {}", fields.join(" → "))
    }
}

pub(super) fn builder_progress(line: &str) -> Option<NixProgress> {
    curl_progress(line).or_else(|| git_progress(line))
}

pub(super) fn curl_progress(line: &str) -> Option<NixProgress> {
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

pub(super) fn git_progress(line: &str) -> Option<NixProgress> {
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

pub(super) fn parse_size(value: &str) -> Option<u64> {
    let (number, multiplier) = match value.chars().last()? {
        'k' => (&value[..value.len() - 1], 1024_f64),
        'M' => (&value[..value.len() - 1], 1024_f64.powi(2)),
        'G' => (&value[..value.len() - 1], 1024_f64.powi(3)),
        'T' => (&value[..value.len() - 1], 1024_f64.powi(4)),
        _ => (value, 1.0),
    };
    Some((number.parse::<f64>().ok()? * multiplier).round() as u64)
}

pub(crate) fn activity_message_level(activity: &str) -> MessageLevel {
    let activity = activity.trim_start().to_ascii_lowercase();
    if ["error:", "fatal:"].iter().any(|prefix| activity.starts_with(prefix)) {
        MessageLevel::Failure
    } else {
        MessageLevel::Info
    }
}

/// 显示与失败报告统一移除终端控制序列、URL 凭证、查询参数和片段。
pub(crate) fn sanitize_activity(activity: &str) -> String {
    static URL: OnceLock<regex::Regex> = OnceLock::new();
    URL.get_or_init(|| {
        regex::Regex::new(
            r"(?P<scheme>[A-Za-z][A-Za-z0-9+.-]*://)(?:[^/@\s]+@)?(?P<host>[^/?#\s]+)(?P<path>/[^?#\s]*)?(?:\?[^#\s]*)?(?:#[^\s]*)?",
        )
        .expect("the static URL regex must be valid")
    })
    .replace_all(&terminal_text(activity), "$scheme$host$path")
    .into_owned()
}

pub(crate) fn activity_summary(text: &str) -> String {
    let text = sanitize_activity(text);
    text.lines()
        .find(|line| line.trim_start().starts_with("error:"))
        .or_else(|| text.lines().find(|line| !line.trim().is_empty()))
        .unwrap_or("")
        .trim()
        .to_owned()
}

/// 将子进程输出转成可显示文本，按回车覆盖当前行的语义保留最后一份进度。
pub(crate) fn terminal_text(text: &str) -> String {
    static ANSI: OnceLock<regex::Regex> = OnceLock::new();
    let text = ANSI
        .get_or_init(|| {
            regex::Regex::new(r"\x1b(?:\[[0-?]*[ -/]*[@-~]|\][^\x07\x1b]*(?:\x07|\x1b\\)|[PX^_][^\x1b]*\x1b\\|[@-_])")
                .expect("the static ANSI regex must be valid")
        })
        .replace_all(text, "");
    let mut output = String::new();
    let mut line_start = 0;
    let mut carriage_return = false;
    for character in text.chars() {
        match character {
            '\r' => carriage_return = true,
            '\n' => {
                output.push('\n');
                line_start = output.len();
                carriage_return = false;
            }
            character if character == '\t' || !character.is_control() => {
                if carriage_return {
                    output.truncate(line_start);
                    carriage_return = false;
                }
                if character == '\t' {
                    output.push_str("    ");
                } else {
                    output.push(character);
                }
            }
            _ => {}
        }
    }
    output
}
