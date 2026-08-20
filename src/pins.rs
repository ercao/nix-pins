//! Pins File 的读写。公共 schema，与 pins.nix reader 同步演进（ADR-0002、ADR-0006）。

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct PinsFile {
    #[serde(rename = "schemaVersion")]
    pub schema_version: u32,
    /// BTreeMap 保证输出键序稳定，使 diff 可读。
    pub pins: BTreeMap<String, Pin>,
    /// 上一轮更新的失败项。Pin 条目本身仍保持可构建的旧内容（ADR-0007）。
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub failures: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pin {
    pub version: String,
    pub fetcher: Fetcher,
    pub hash: String,
    /// 未使用的能力整个键缺失，而非为 null（ADR-0006）。
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub derived: BTreeMap<String, String>,
    /// 带假哈希的中间 FOD drvPath，用作重算判据（ADR-0014）。
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub fingerprints: BTreeMap<String, String>,
}

/// 带标签的 fetcher，每种只出现自身字段（ADR-0006）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Fetcher {
    Github(BTreeMap<String, serde_json::Value>),
    Git(BTreeMap<String, serde_json::Value>),
    Url(BTreeMap<String, serde_json::Value>),
}

impl PinsFile {
    pub fn load(path: &std::path::Path) -> std::io::Result<Self> {
        match std::fs::read(path) {
            Ok(b) => serde_json::from_slice(&b)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e)),
            // 首轮运行：文件不存在等价于空集，阶段一无需它（ADR-0015）。
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self {
                schema_version: SCHEMA_VERSION,
                pins: BTreeMap::new(),
                failures: BTreeMap::new(),
            }),
            Err(e) => Err(e),
        }
    }

    pub fn save(&self, path: &std::path::Path) -> std::io::Result<()> {
        let mut s = serde_json::to_string_pretty(self)?;
        s.push('\n');
        std::fs::write(path, s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> PinsFile {
        let mut pins = BTreeMap::new();
        let mut gh = BTreeMap::new();
        gh.insert("owner".into(), "rs".into());
        gh.insert("repo".into(), "curlie".into());
        gh.insert("rev".into(), "v1.8.2".into());
        pins.insert(
            "curlie".into(),
            Pin {
                version: "v1.8.2".into(),
                fetcher: Fetcher::Github(gh),
                hash: "sha256-BlpIDik4hkU4c+KCyAmgUURIN362RDQID/qo6Ojp2Ek=".into(),
                derived: BTreeMap::new(),
                fingerprints: BTreeMap::new(),
            },
        );
        PinsFile {
            schema_version: SCHEMA_VERSION,
            pins,
            failures: BTreeMap::new(),
        }
    }

    /// 空 derived 不得序列化为 null 或空对象（ADR-0006）。
    #[test]
    fn omits_empty_capabilities() {
        let s = serde_json::to_string(&sample()).unwrap();
        assert!(!s.contains("derived"), "{s}");
        assert!(!s.contains("null"), "{s}");
    }

    #[test]
    fn roundtrips() {
        let s = serde_json::to_string(&sample()).unwrap();
        let back: PinsFile = serde_json::from_str(&s).unwrap();
        assert_eq!(back.pins["curlie"].version, "v1.8.2");
        assert!(matches!(back.pins["curlie"].fetcher, Fetcher::Github(_)));
    }
}
