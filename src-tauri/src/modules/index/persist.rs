use std::path::Path;

use crate::modules::index::symbols::Symbol;

const MAGIC: &[u8; 8] = b"KENIDX01";
const CURRENT_VERSION: u32 = 1;
const HEADER_LEN: usize = 16;

#[derive(Clone, Debug, PartialEq, Eq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct PersistedFile {
    pub path: String,
    pub mtime_ms: u64,
    pub symbols: Vec<Symbol>,
}

#[derive(Clone, Debug, PartialEq, Eq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct PersistedIndex {
    pub version: u32,
    pub root: String,
    pub files: Vec<PersistedFile>,
}

fn tmp_path(path: &Path) -> std::path::PathBuf {
    let mut name = path.file_name().map(|s| s.to_os_string()).unwrap_or_default();
    name.push(".tmp");
    path.with_file_name(name)
}

pub fn save(path: &Path, index: &PersistedIndex) -> std::io::Result<()> {
    let body = rkyv::to_bytes::<rkyv::rancor::Error>(index)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
    let crc = crc32fast::hash(&body);
    let mut buf = Vec::with_capacity(HEADER_LEN + body.len());
    buf.extend_from_slice(MAGIC);
    buf.extend_from_slice(&CURRENT_VERSION.to_le_bytes());
    buf.extend_from_slice(&crc.to_le_bytes());
    buf.extend_from_slice(&body);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = tmp_path(path);
    std::fs::write(&tmp, &buf)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

pub fn load(path: &Path) -> Option<PersistedIndex> {
    let file = std::fs::File::open(path).ok()?;
    let mmap = unsafe { memmap2::Mmap::map(&file).ok()? };
    let bytes: &[u8] = &mmap;
    if bytes.len() < HEADER_LEN || &bytes[0..8] != MAGIC {
        return None;
    }
    let version = u32::from_le_bytes(bytes[8..12].try_into().ok()?);
    if version != CURRENT_VERSION {
        return None;
    }
    let crc = u32::from_le_bytes(bytes[12..16].try_into().ok()?);
    let body = &bytes[HEADER_LEN..];
    if crc32fast::hash(body) != crc {
        return None;
    }
    let archived = rkyv::access::<ArchivedPersistedIndex, rkyv::rancor::Error>(body).ok()?;
    rkyv::deserialize::<PersistedIndex, rkyv::rancor::Error>(archived).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::index::symbols::SymbolKind;

    fn sample() -> PersistedIndex {
        PersistedIndex {
            version: CURRENT_VERSION,
            root: "/proj".to_string(),
            files: vec![PersistedFile {
                path: "/proj/a.ts".to_string(),
                mtime_ms: 1234,
                symbols: vec![Symbol {
                    name: "greet".to_string(),
                    kind: SymbolKind::Function,
                    start_line: 1,
                    end_line: 3,
                }],
            }],
        }
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index.kenidx");
        let index = sample();
        save(&path, &index).unwrap();
        assert_eq!(load(&path), Some(index));
    }

    #[test]
    fn atomic_write_leaves_no_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index.kenidx");
        save(&path, &sample()).unwrap();
        assert!(path.exists());
        assert!(!tmp_path(&path).exists());
    }

    #[test]
    fn missing_file_loads_none() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load(&dir.path().join("nope.kenidx")), None);
    }

    #[test]
    fn version_mismatch_loads_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index.kenidx");
        save(&path, &sample()).unwrap();
        let mut bytes = std::fs::read(&path).unwrap();
        bytes[8] = bytes[8].wrapping_add(1);
        std::fs::write(&path, &bytes).unwrap();
        assert_eq!(load(&path), None);
    }

    #[test]
    fn corrupted_body_loads_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index.kenidx");
        save(&path, &sample()).unwrap();
        let mut bytes = std::fs::read(&path).unwrap();
        let last = bytes.len() - 1;
        bytes[last] = bytes[last].wrapping_add(1);
        std::fs::write(&path, &bytes).unwrap();
        assert_eq!(load(&path), None);
    }

    #[test]
    fn truncated_header_loads_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index.kenidx");
        std::fs::write(&path, MAGIC).unwrap();
        assert_eq!(load(&path), None);
    }
}
