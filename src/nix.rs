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
        return Err(Error::Nix(
            String::from_utf8_lossy(&out.stderr).into_owned(),
        ));
    }
    serde_json::from_slice(&out.stdout).map_err(|e| Error::Nix(e.to_string()))
}

/// 构建一个注定失败的 FOD，从失败输出中取回真实哈希（ADR-0003、ADR-0011）。
pub fn resolve_hash(drv_path: &str) -> Result<String, Error> {
    let out = Command::new("nix")
        .args(["build", "--no-link", "-L", &format!("{drv_path}^out")])
        .output()
        .map_err(|e| Error::Nix(e.to_string()))?;
    // 退出码不可作为判据：404 的 fetchurl 曾以 0 退出且无 got 行（ADR-0003）。
    let mut log = String::from_utf8_lossy(&out.stderr).into_owned();
    if !out.stdout.is_empty() {
        if !log.is_empty() && !log.ends_with('\n') {
            log.push('\n');
        }
        log.push_str(&String::from_utf8_lossy(&out.stdout));
    }
    parse_got(&log).ok_or(Error::NoGotLine(log))
}

/// 从 hash mismatch 输出中提取 got 行的哈希。
fn parse_got(log: &str) -> Option<String> {
    log.lines()
        .find_map(|l| l.split_once("got:"))
        .map(|(_, rest)| rest.trim().to_string())
        .filter(|h| h.starts_with("sha256-") || h.starts_with("sha512-"))
}

#[cfg(test)]
mod tests {
    use super::parse_got;

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
}
