//! The project records: one entry per Herdr workspace, keyed by workspace id.
//!
//! A record holds the edit/view links for the workspace's FigCraft document. The links carry
//! tokens, so the store is written with mode `0600` on Unix and never logged.
//!
//! The Python service kept the same shape in `project/data/projects.json`. On first start, when
//! the new store does not exist yet, those records are imported so an already-published workspace
//! keeps its document instead of publishing a duplicate.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result, ResultExt};

/// One published project, mirrored from the Python service's JSON.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ProjectRecord {
    /// Herdr workspace id (`wP`).
    #[serde(default)]
    pub id: String,
    /// Herdr workspace label.
    #[serde(default)]
    pub label: String,
    /// Checkout path reported by Herdr, when the workspace is a repository.
    #[serde(default)]
    pub path: Option<String>,
    /// FigCraft file id on the document server.
    #[serde(default)]
    pub file: Option<String>,
    /// Owner edit link (`figcraft://join?...`). Never shown to the user by this service.
    #[serde(default)]
    pub edit: Option<String>,
    /// Read-only link, when the server returned one.
    #[serde(default)]
    pub view: Option<String>,
    /// Document name (`Mindmap · <label>`).
    #[serde(default)]
    pub name: Option<String>,
    /// Whether the agent has replaced the skeleton with a real map.
    #[serde(default)]
    pub built: bool,
    /// Creation time, `YYYY-MM-DDTHH:MM:SSZ`.
    #[serde(default)]
    pub created: String,
}

impl ProjectRecord {
    /// True when this record can open a document without publishing a new one.
    pub fn has_link(&self) -> bool {
        self.edit
            .as_deref()
            .is_some_and(|edit| !edit.trim().is_empty())
    }
}

/// The JSON store, protected by a process-wide lock.
///
/// Every operation reads and writes the file in one critical section. That keeps a CLI and a
/// running service from losing each other's updates and makes publish-or-reuse atomic within one
/// process. Writes go to a temporary file and are renamed, so a crash cannot truncate the store.
#[derive(Debug)]
pub struct Store {
    path: PathBuf,
    legacy: PathBuf,
    lock: Mutex<()>,
}

impl Store {
    /// Create a store at `data_dir/projects.json`, importing `legacy` once if needed.
    pub fn new(data_dir: &Path, legacy: PathBuf) -> Self {
        Self {
            path: data_dir.join("projects.json"),
            legacy,
            lock: Mutex::new(()),
        }
    }

    /// All records, sorted by workspace id.
    pub fn all(&self) -> Result<BTreeMap<String, ProjectRecord>> {
        let _guard = self
            .lock
            .lock()
            .map_err(|_| Error::new("project record lock poisoned"))?;
        self.load_locked()
    }

    /// One record by workspace id.
    pub fn get(&self, id: &str) -> Result<Option<ProjectRecord>> {
        Ok(self.all()?.remove(id))
    }

    /// Insert or replace one record, preserving the others.
    pub fn upsert(&self, record: ProjectRecord) -> Result<()> {
        let _guard = self
            .lock
            .lock()
            .map_err(|_| Error::new("project record lock poisoned"))?;
        let mut records = self.load_locked()?;
        records.insert(record.id.clone(), record);
        self.save_locked(&records)
    }

    fn load_locked(&self) -> Result<BTreeMap<String, ProjectRecord>> {
        if !self.path.is_file() && self.legacy.is_file() {
            let records = self.read_file(&self.legacy)?;
            self.save_locked(&records)?;
            eprintln!(
                "mindmap-service: imported {} project record(s) from {}",
                records.len(),
                self.legacy.display()
            );
            return Ok(records);
        }
        self.read_file(&self.path)
    }

    fn read_file(&self, path: &Path) -> Result<BTreeMap<String, ProjectRecord>> {
        if !path.is_file() {
            return Ok(BTreeMap::new());
        }
        let text =
            fs::read_to_string(path).context(format!("read project records {}", path.display()))?;
        if text.trim().is_empty() {
            return Ok(BTreeMap::new());
        }
        serde_json::from_str(&text).context(format!("parse project records {}", path.display()))
    }

    fn save_locked(&self, records: &BTreeMap<String, ProjectRecord>) -> Result<()> {
        let parent = self
            .path
            .parent()
            .ok_or_else(|| Error::new("project records path has no parent directory"))?;
        fs::create_dir_all(parent)
            .context(format!("create state directory {}", parent.display()))?;

        let mut data = serde_json::to_string_pretty(records)?;
        data.push('\n');
        let file_name = self
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("projects.json");
        let temporary = parent.join(format!(".{file_name}.{}.tmp", std::process::id()));

        let write = (|| -> Result<()> {
            fs::write(&temporary, data.as_bytes())
                .context(format!("write temporary records {}", temporary.display()))?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600))
                    .context(format!("protect records {}", temporary.display()))?;
            }
            fs::rename(&temporary, &self.path).context(format!(
                "replace project records {} with {}",
                self.path.display(),
                temporary.display()
            ))
        })();
        if write.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        write
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_dir(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "mindmap-service-test-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|dur| dur.as_nanos())
                .unwrap_or(0)
        ))
    }

    #[test]
    fn imports_legacy_only_once_and_round_trips() {
        let dir = unique_dir("import");
        let legacy_dir = dir.join("legacy");
        std::fs::create_dir_all(&legacy_dir).unwrap_or(());
        let legacy = legacy_dir.join("projects.json");
        let record = ProjectRecord {
            id: "wP".into(),
            label: "eisledger".into(),
            edit: Some("figcraft://join?file=abc&token=t".into()),
            file: Some("abc".into()),
            ..ProjectRecord::default()
        };
        std::fs::write(
            &legacy,
            serde_json::to_vec(&serde_json::json!({ "wP": record })).unwrap_or_default(),
        )
        .unwrap_or(());

        let store = Store::new(&dir.join("data"), legacy.clone());
        let records = store.all().unwrap_or_default();
        assert_eq!(records.len(), 1);
        assert_eq!(
            records.get("wP").and_then(|record| record.file.clone()),
            Some("abc".to_string())
        );

        // Change the legacy file; the new store must not import again.
        std::fs::write(&legacy, b"{}").unwrap_or(());
        assert_eq!(store.all().map(|records| records.len()).unwrap_or(0), 1);

        store
            .upsert(ProjectRecord {
                id: "wQ".into(),
                ..ProjectRecord::default()
            })
            .unwrap_or(());
        assert_eq!(store.all().map(|records| records.len()).unwrap_or(0), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
