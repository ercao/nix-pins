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

/// 求值一个 Nix 表达式并以 JSON 返回。
pub fn eval_json(expr: &str) -> Result<serde_json::Value, Error> {
    let out = Command::new("nix")
        .args(["eval", "--impure", "--json", "--expr", expr])
        .output()
        .map_err(|e| Error::Nix(e.to_string()))?;
    if !out.status.success() {
        return Err(Error::Nix(String::from_utf8_lossy(&out.stderr).into_owned()));
    }
    serde_json::from_slice(&out.stdout).map_err(|e| Error::Nix(e.to_string()))
}

#[derive(Debug)]
enum Activity {
    Copy,
    Download { done: u64, total: Option<u64> },
    Build { phase: Option<(u64, String)> },
    Query,
}

#[derive(Default)]
struct NixLog {
    activities: std::collections::BTreeMap<u64, Activity>,
    phase_sequence: u64,
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
                    Some(100) => Activity::Copy,
                    Some(101) => Activity::Download { done: 0, total: None },
                    Some(105) => Activity::Build { phase: None },
                    Some(108) => Activity::Query,
                    Some(_) => return None,
                    None => return Some(line.into()),
                };
                self.activities.insert(id, activity);
            }
            "stop" => {
                self.activities.remove(&id);
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
                    (104, Some(Activity::Build { phase }), Some(fields)) => {
                        let Some(value) = fields.first().and_then(serde_json::Value::as_str) else {
                            return Some(line.into());
                        };
                        self.phase_sequence += 1;
                        *phase = Some((self.phase_sequence, value.into()));
                    }
                    (101, _, Some(fields)) => {
                        return fields
                            .first()
                            .and_then(serde_json::Value::as_str)
                            .map(str::to_owned)
                            .or_else(|| Some(line.into()));
                    }
                    (101 | 104 | 105, _, None) => return Some(line.into()),
                    _ => return None,
                }
            }
            _ => unreachable!(),
        }
        None
    }

    fn detail(&self) -> Option<String> {
        let downloads: Vec<_> = self
            .activities
            .values()
            .filter_map(|activity| match activity {
                Activity::Download { done, total } => Some((*done, *total)),
                _ => None,
            })
            .collect();
        if !downloads.is_empty() {
            let done = downloads.iter().map(|(done, _)| done).sum::<u64>();
            let transfers = downloads.len();
            if downloads.iter().all(|(_, total)| total.is_some()) {
                let total = downloads.iter().filter_map(|(_, total)| *total).sum::<u64>();
                let percent = done.saturating_mul(100).checked_div(total).unwrap_or(0);
                return Some(format!(
                    "Downloading {}/{} MiB ({percent}%) · {transfers} {}",
                    format_mib(done),
                    format_mib(total),
                    if transfers == 1 { "transfer" } else { "transfers" }
                ));
            }
            return Some(format!(
                "Downloading {} downloaded · {transfers} {}",
                format_bytes(done),
                if transfers == 1 { "transfer" } else { "transfers" }
            ));
        }
        if self
            .activities
            .values()
            .any(|activity| matches!(activity, Activity::Copy))
        {
            return Some("Copying Store Path".into());
        }
        if let Some((_, phase)) = self
            .activities
            .values()
            .filter_map(|activity| match activity {
                Activity::Build { phase } => phase.as_ref(),
                _ => None,
            })
            .max_by_key(|(sequence, _)| *sequence)
        {
            return Some(phase.clone());
        }
        if self
            .activities
            .values()
            .any(|activity| matches!(activity, Activity::Build { .. }))
        {
            return Some("Building".into());
        }
        self.activities
            .values()
            .any(|activity| matches!(activity, Activity::Query))
            .then(|| "Querying Cache".into())
    }
}

fn format_bytes(bytes: u64) -> String {
    format!("{} MiB", format_mib(bytes))
}

fn format_mib(bytes: u64) -> String {
    const MIB: f64 = 1024.0 * 1024.0;
    format!("{:.1}", bytes as f64 / MIB)
}

pub fn resolve_hash(drv_path: &str, mut progress: impl FnMut(Option<String>)) -> Result<String, Error> {
    use std::io::{BufRead, BufReader, Read};
    use std::process::Stdio;

    let mut child = Command::new("nix")
        .args([
            "build",
            "--no-link",
            "-L",
            "--log-format",
            "internal-json",
            &format!("{drv_path}^out"),
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| Error::Nix(error.to_string()))?;
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
    let mut state = NixLog::default();
    let mut last_detail = None;
    let mut diagnostics = String::new();
    let mut stderr_error = None;
    for line in BufReader::new(stderr).lines() {
        let line = match line {
            Ok(line) => line,
            Err(error) => {
                stderr_error = Some(error);
                break;
            }
        };
        if let Some(line) = state.push(&line) {
            diagnostics.push_str(&line);
            diagnostics.push('\n');
        }
        let detail = state.detail();
        if detail != last_detail {
            progress(detail.clone());
            last_detail = detail;
        }
    }
    if stderr_error.is_some() {
        let _ = child.kill();
    }
    let wait_result = child.wait();
    let stdout = stdout.join();
    progress(None);

    if let Some(error) = stderr_error {
        return Err(Error::Nix(error.to_string()));
    }
    wait_result.map_err(|error| Error::Nix(error.to_string()))?;
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
    parse_got(&diagnostics).ok_or(Error::NoGotLine(diagnostics))
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
    use super::{parse_got, NixLog};

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

        let detail = log.detail().unwrap();
        assert_eq!(detail, "Downloading 1.5/3.0 MiB (50%) · 2 transfers");
        assert!(!detail.contains("secret.invalid"));
    }

    #[test]
    fn internal_json_omits_percentage_when_any_download_total_is_unknown() {
        let mut log = NixLog::default();
        log.push(r#"@nix {"action":"start","id":1,"type":101,"fields":["hidden"]}"#);
        log.push(r#"@nix {"action":"result","id":1,"type":105,"fields":[1048576,0,0,0]}"#);

        assert_eq!(
            log.detail().as_deref(),
            Some("Downloading 1.0 MiB downloaded · 1 transfer")
        );
    }

    #[test]
    fn internal_json_uses_fixed_detail_priority_and_preserves_diagnostics() {
        let mut log = NixLog::default();

        log.push(r#"@nix {"action":"start","id":1,"type":108,"fields":["cache"]}"#);
        assert_eq!(log.detail().as_deref(), Some("Querying Cache"));
        log.push(r#"@nix {"action":"start","id":2,"type":105,"fields":["/nix/store/demo.drv"]}"#);
        assert_eq!(log.detail().as_deref(), Some("Building"));
        log.push(r#"@nix {"action":"result","id":2,"type":104,"fields":["buildPhase"]}"#);
        assert_eq!(log.detail().as_deref(), Some("buildPhase"));
        log.push(r#"@nix {"action":"start","id":5,"type":105,"fields":["/nix/store/newer.drv"]}"#);
        log.push(r#"@nix {"action":"result","id":5,"type":104,"fields":["installPhase"]}"#);
        assert_eq!(log.detail().as_deref(), Some("installPhase"));
        log.push(r#"@nix {"action":"stop","id":5}"#);
        assert_eq!(log.detail().as_deref(), Some("buildPhase"));
        log.push(r#"@nix {"action":"start","id":3,"type":100,"fields":["from","to"]}"#);
        assert_eq!(log.detail().as_deref(), Some("Copying Store Path"));
        log.push(r#"@nix {"action":"start","id":4,"type":101,"fields":["https://secret.invalid"]}"#);
        assert!(log.detail().unwrap().starts_with("Downloading"));

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
}
