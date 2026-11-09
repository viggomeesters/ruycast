//! Compatible successful-choice persistence. No query or argument recording.
use crate::{catalog::bounded_recency, private_storage, profile::SaveReceipt};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs::OpenOptions,
    io::Read,
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
};

#[derive(Serialize, Deserialize)]
struct State {
    schema: u32,
    entries: HashMap<String, f64>,
}
pub struct RankingStore {
    file: PathBuf,
    entries: HashMap<String, f64>,
    pub revision: u64,
}
impl RankingStore {
    pub fn open(root: &Path, now: f64) -> Self {
        let file = root.join("search-ranking.json");
        let load = || -> Option<HashMap<String, f64>> {
            let bytes = read_bytes(&file)?;
            let state: State = serde_json::from_slice(&bytes).ok()?;
            (state.schema == 1).then(|| bounded_recency(state.entries, now))
        };
        let entries = load().unwrap_or_default();
        Self {
            file,
            entries,
            revision: 0,
        }
    }
    pub fn entries(&self) -> &HashMap<String, f64> {
        &self.entries
    }
    pub fn record_success(&mut self, id: &str, now: f64) -> Result<SaveReceipt, String> {
        if id.is_empty() || id.len() > 512 || !now.is_finite() || now < 0. {
            return Err("Invalid successful choice".into());
        }
        let mut entries = self.entries.clone();
        entries.insert(id.into(), now);
        self.publish(bounded_recency(entries, now))
    }
    pub fn reset(&mut self) -> Result<SaveReceipt, String> {
        self.publish(HashMap::new())
    }
    fn publish(&mut self, entries: HashMap<String, f64>) -> Result<SaveReceipt, String> {
        let state = State { schema: 1, entries };
        let bytes = serde_json::to_vec_pretty(&state).map_err(|e| e.to_string())?;
        let receipt =
            SaveReceipt::from_outcome(private_storage::write_outcome(&self.file, &bytes, true)?);
        if read_bytes(&self.file).as_deref() != Some(bytes.as_slice()) {
            return Err("Ranking changed after publication".into());
        }
        self.entries = state.entries;
        self.revision = self.revision.wrapping_add(1);
        Ok(receipt)
    }
}

fn read_bytes(path: &Path) -> Option<Vec<u8>> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .ok()?;
    let metadata = file.metadata().ok()?;
    if !metadata.is_file() || metadata.len() > 1_000_000 {
        return None;
    }
    let mut bytes = vec![];
    file.take(1_000_001).read_to_end(&mut bytes).ok()?;
    (bytes.len() <= 1_000_000).then_some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    #[test]
    fn roundtrip_schema_retention_reset_and_no_query_fields() {
        let root = std::env::temp_dir().join(format!(
            "ruycast-ranking-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        private_storage::secure_directory(&root).unwrap();
        let mut ranking = RankingStore::open(&root, 10_000_000.);
        ranking.record_success("fixture.one", 10_000_000.).unwrap();
        assert!(RankingStore::open(&root, 10_000_001.)
            .entries()
            .contains_key("fixture.one"));
        assert!(RankingStore::open(&root, 20_000_000.).entries().is_empty());
        let value: serde_json::Value =
            serde_json::from_slice(&fs::read(root.join("search-ranking.json")).unwrap()).unwrap();
        assert_eq!(value.as_object().unwrap().len(), 2);
        assert!(value.get("query").is_none());
        ranking.reset().unwrap();
        assert!(RankingStore::open(&root, 10_000_001.).entries().is_empty());
        fs::write(root.join("search-ranking.json"), b"broken").unwrap();
        assert!(RankingStore::open(&root, 0.).entries().is_empty());
        let fifo = root.join("fifo");
        let name = std::ffi::CString::new(fifo.as_os_str().as_encoded_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        assert!(read_bytes(&fifo).is_none());
        fs::remove_dir_all(&root).unwrap();
    }
}
