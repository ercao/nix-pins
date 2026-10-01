//! Pins File 的读写。公共 schema，与 nix/pins.nix reader 同步演进（ADR-0002）。

use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

pub const SCHEMA_VERSION: u32 = 2;

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct PinsFile {
    #[serde(rename = "schemaVersion")]
    pub schema_version: u32,
    /// BTreeMap 保证输出键序稳定，使 diff 可读。
    pub pins: BTreeMap<String, Pin>,
    /// 上一轮更新的失败项。Pin 条目本身仍保持可构建的旧内容（ADR-0002）。
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub failures: BTreeMap<String, String>,
}

/// 生命周期覆盖读取、更新和原子保存，持有独立锁文件以免 rename 替换目标后失去互斥。
pub struct Transaction {
    path: PathBuf,
    lock: File,
    original: Option<Vec<u8>>,
    pub pins: PinsFile,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pin {
    pub version: String,
    pub sources: BTreeMap<String, Source>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Source {
    pub fetcher: Fetcher,
    pub hash: String,
    /// 未使用的能力整个键缺失，而非为 null（ADR-0002）。
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub derived: BTreeMap<String, String>,
    /// 带假哈希的中间 FOD drvPath，用作重算判据（ADR-0003）。
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub fingerprints: BTreeMap<String, String>,
}

/// 带标签的 fetcher，每种只出现自身字段（ADR-0002）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Fetcher {
    Github(BTreeMap<String, serde_json::Value>),
    Git(BTreeMap<String, serde_json::Value>),
    Huggingface(BTreeMap<String, serde_json::Value>),
    Url(BTreeMap<String, serde_json::Value>),
    Zip(BTreeMap<String, serde_json::Value>),
}

impl Fetcher {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Github(_) => "GitHub Fetcher",
            Self::Git(_) => "git Fetcher",
            Self::Huggingface(_) => "Hugging Face Fetcher",
            Self::Url(_) => "URL Fetcher",
            Self::Zip(_) => "zip Fetcher",
        }
    }
}

impl PinsFile {
    pub fn load(path: &Path) -> std::io::Result<Self> {
        match std::fs::read(path) {
            Ok(bytes) => Self::from_bytes(&bytes),
            // 首轮运行：文件不存在等价于空集，阶段一无需它（ADR-0016）。
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self {
                schema_version: SCHEMA_VERSION,
                pins: BTreeMap::new(),
                failures: BTreeMap::new(),
            }),
            Err(e) => Err(e),
        }
    }

    /// 先取得目标路径对应的独占锁，再读取快照；错误返回时文件句柄自动释放锁。
    pub fn transaction(path: &Path) -> std::io::Result<Transaction> {
        let lock_path = lock_path(path)?;
        let lock = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(lock_path)?;
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
    /// 内容未变时保留 mtime；在同目录写完临时文件后 rename，读者只能看到完整 JSON。
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

fn lock_path(path: &Path) -> std::io::Result<PathBuf> {
    let name = path
        .file_name()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "Pins file path has no file name"))?;
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    // 只规范化父目录，首次创建和原子替换后仍使用同一把锁。
    let target = std::fs::canonicalize(parent)?.join(name);
    // 固定 FNV-1a 算法，避免不同 Rust 版本改变路径到锁名的映射。
    let hash = target
        .as_os_str()
        .as_bytes()
        .iter()
        .fold(0xcbf29ce484222325_u64, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
        });

    let uid = unsafe { libc::geteuid() };
    let directory = PathBuf::from(format!("/tmp/nix-pins-locks-{uid}"));
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&directory)?;
    let metadata = std::fs::symlink_metadata(&directory)?;
    if !metadata.is_dir() || metadata.uid() != uid || metadata.mode() & 0o077 != 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "lock directory must be private and owned by the current user",
        ));
    }
    // 锁文件不可在释放锁时删除，否则等待中的进程可能锁住不同 inode。
    Ok(directory.join(format!("{hash:016x}.lock")))
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
                sources: BTreeMap::from([(
                    "default".into(),
                    Source {
                        fetcher: Fetcher::Github(gh),
                        hash: "sha256-BlpIDik4hkU4c+KCyAmgUURIN362RDQID/qo6Ojp2Ek=".into(),
                        derived: BTreeMap::new(),
                        fingerprints: BTreeMap::new(),
                    },
                )]),
            },
        );
        PinsFile {
            schema_version: SCHEMA_VERSION,
            pins,
            failures: BTreeMap::new(),
        }
    }

    /// 空 derived 不得序列化为 null 或空对象（ADR-0002）。
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
        assert!(matches!(
            back.pins["curlie"].sources["default"].fetcher,
            Fetcher::Github(_)
        ));
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
        let lock = lock_path(&path).unwrap();
        assert!(lock.starts_with(format!("/tmp/nix-pins-locks-{}", unsafe { libc::geteuid() })));
        assert_eq!(lock, lock_path(&root.join("./pins.json")).unwrap());
        assert_ne!(lock, lock_path(&root.join("other.json")).unwrap());
        std::fs::create_dir(root.join("other")).unwrap();
        assert_ne!(lock, lock_path(&root.join("other/pins.json")).unwrap());
        std::os::unix::fs::symlink(&root, root.join("alias")).unwrap();
        let alias = root.join("alias/pins.json");
        assert_eq!(lock, lock_path(&alias).unwrap());

        let mut first = PinsFile::transaction(&path).unwrap();
        assert!(!path.with_file_name("pins.json.lock").exists());
        first.pins = sample();
        first.save().unwrap();
        assert_eq!(lock, lock_path(&path).unwrap());
        let modified = std::fs::metadata(&path).unwrap().modified().unwrap();

        let competing = OpenOptions::new()
            .read(true)
            .write(true)
            .open(lock_path(&alias).unwrap())
            .unwrap();
        assert!(competing.try_lock_exclusive().is_err());
        drop(first);
        assert!(lock.is_file());
        competing.try_lock_exclusive().unwrap();
        FileExt::unlock(&competing).unwrap();
        drop(competing);

        std::thread::sleep(std::time::Duration::from_millis(20));
        let mut unchanged = PinsFile::transaction(&path).unwrap();
        unchanged.save().unwrap();
        drop(unchanged);
        assert_eq!(std::fs::metadata(&path).unwrap().modified().unwrap(), modified);
        let loaded = PinsFile::load(&path).unwrap();
        assert_eq!(loaded.pins["curlie"].version, "v1.8.2");

        std::fs::remove_file(lock).unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn readers_only_observe_complete_json() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

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
