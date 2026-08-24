//! Pins File 的读写。公共 schema，与 pins.nix reader 同步演进（ADR-0002、ADR-0006）。

use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

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

pub struct Transaction {
    path: PathBuf,
    lock: File,
    original: Option<Vec<u8>>,
    pub pins: PinsFile,
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
    Huggingface(BTreeMap<String, serde_json::Value>),
    Url(BTreeMap<String, serde_json::Value>),
    Zip(BTreeMap<String, serde_json::Value>),
}

impl PinsFile {
    pub fn load(path: &Path) -> std::io::Result<Self> {
        match std::fs::read(path) {
            Ok(bytes) => Self::from_bytes(&bytes),
            // 首轮运行：文件不存在等价于空集，阶段一无需它（ADR-0015）。
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self {
                schema_version: SCHEMA_VERSION,
                pins: BTreeMap::new(),
                failures: BTreeMap::new(),
            }),
            Err(e) => Err(e),
        }
    }

    pub fn transaction(path: &Path) -> std::io::Result<Transaction> {
        let lock_path = lock_path(path);
        let lock = OpenOptions::new().create(true).read(true).write(true).open(lock_path)?;
        lock.lock_exclusive()?;
        let original = match std::fs::read(path) {
            Ok(bytes) => Some(bytes),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error),
        };
        let pins = match &original {
            Some(bytes) => Self::from_bytes(bytes)?,
            None => Self::empty(),
        };
        Ok(Transaction {
            path: path.to_owned(),
            lock,
            original,
            pins,
        })
    }

    fn empty() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            pins: BTreeMap::new(),
            failures: BTreeMap::new(),
        }
    }

    fn from_bytes(bytes: &[u8]) -> std::io::Result<Self> {
        let pins: Self = serde_json::from_slice(bytes)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
        if pins.schema_version != SCHEMA_VERSION {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "unsupported Pins File schemaVersion {}; expected {SCHEMA_VERSION}",
                    pins.schema_version
                ),
            ));
        }
        Ok(pins)
    }

    fn to_bytes(&self) -> std::io::Result<Vec<u8>> {
        let mut bytes = serde_json::to_vec_pretty(self)?;
        bytes.push(b'\n');
        Ok(bytes)
    }
}

impl Transaction {
    pub fn save(&mut self) -> std::io::Result<()> {
        let bytes = self.pins.to_bytes()?;
        if self.original.as_deref() == Some(bytes.as_slice()) {
            return Ok(());
        }
        let temp_path = temporary_path(&self.path);
        let result = (|| {
            let mut temp = OpenOptions::new().create_new(true).write(true).open(&temp_path)?;
            temp.write_all(&bytes)?;
            temp.sync_all()?;
            drop(temp);
            std::fs::rename(&temp_path, &self.path)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&temp_path);
        } else {
            self.original = Some(bytes);
        }
        result
    }
}

impl Drop for Transaction {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.lock);
    }
}

fn lock_path(path: &Path) -> PathBuf {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    path.with_file_name(format!("{name}.lock"))
}

fn temporary_path(path: &Path) -> PathBuf {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    path.with_file_name(format!(".{name}.{}.{nonce}.tmp", std::process::id()))
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

    #[test]
    fn transaction_serializes_access_and_preserves_unchanged_mtime() {
        let root = std::env::temp_dir().join(format!(
            "nix-pins-transaction-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("pins.json");

        let mut first = PinsFile::transaction(&path).unwrap();
        first.pins = sample();
        first.save().unwrap();
        let modified = std::fs::metadata(&path).unwrap().modified().unwrap();

        let competing = OpenOptions::new()
            .read(true)
            .write(true)
            .open(lock_path(&path))
            .unwrap();
        assert!(competing.try_lock_exclusive().is_err());
        drop(first);
        competing.try_lock_exclusive().unwrap();
        FileExt::unlock(&competing).unwrap();

        std::thread::sleep(std::time::Duration::from_millis(20));
        let mut unchanged = PinsFile::transaction(&path).unwrap();
        unchanged.save().unwrap();
        drop(unchanged);
        assert_eq!(std::fs::metadata(&path).unwrap().modified().unwrap(), modified);
        let loaded = PinsFile::load(&path).unwrap();
        assert_eq!(loaded.pins["curlie"].version, "v1.8.2");

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn readers_only_observe_complete_json() {
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        use std::sync::Arc;

        let root = std::env::temp_dir().join(format!(
            "nix-pins-observer-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("pins.json");
        let mut initial = PinsFile::transaction(&path).unwrap();
        initial.pins = sample();
        initial.save().unwrap();
        drop(initial);

        let running = Arc::new(AtomicBool::new(true));
        let observations = Arc::new(AtomicUsize::new(0));
        let reader = {
            let path = path.clone();
            let running = Arc::clone(&running);
            let observations = Arc::clone(&observations);
            std::thread::spawn(move || {
                while running.load(Ordering::Relaxed) {
                    let bytes = std::fs::read(&path).unwrap();
                    serde_json::from_slice::<PinsFile>(&bytes).unwrap();
                    observations.fetch_add(1, Ordering::Relaxed);
                }
            })
        };

        for version in 0..50 {
            let mut transaction = PinsFile::transaction(&path).unwrap();
            transaction.pins.pins.get_mut("curlie").unwrap().version = format!("v{version}");
            transaction.save().unwrap();
        }
        running.store(false, Ordering::Relaxed);
        reader.join().unwrap();
        assert!(observations.load(Ordering::Relaxed) > 0);

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn failed_save_preserves_the_original_file() {
        use std::os::unix::fs::PermissionsExt;

        let root = std::env::temp_dir().join(format!(
            "nix-pins-failed-save-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("pins.json");
        let original = sample().to_bytes().unwrap();
        std::fs::write(&path, &original).unwrap();

        let mut transaction = PinsFile::transaction(&path).unwrap();
        transaction.pins.pins.get_mut("curlie").unwrap().version = "v2".into();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o555)).unwrap();
        let result = transaction.save();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o755)).unwrap();

        assert!(result.is_err());
        assert_eq!(std::fs::read(&path).unwrap(), original);
        drop(transaction);
        std::fs::remove_dir_all(root).unwrap();
    }
}
