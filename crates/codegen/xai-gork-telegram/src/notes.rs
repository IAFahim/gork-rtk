//! Durable notes queue (JSON file) for offline phone work.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Note {
    pub id: String,
    pub text: String,
    pub session_id: Option<String>,
    pub created_at: String,
    #[serde(default)]
    pub media_path: Option<String>,
    #[serde(default)]
    pub is_image: bool,
}

#[derive(Debug, Clone)]
pub struct NoteQueue {
    path: PathBuf,
}

// Clone is fine — path only; file is source of truth

#[derive(Debug, Default, Serialize, Deserialize)]
struct Store {
    notes: Vec<Note>,
}

impl NoteQueue {
    pub fn open(data_dir: &Path) -> Result<Self> {
        fs::create_dir_all(data_dir)?;
        let path = data_dir.join("notes.json");
        if !path.exists() {
            fs::write(&path, r#"{"notes":[]}"#)?;
        }
        Ok(Self { path })
    }

    fn load(&self) -> Result<Store> {
        let raw = fs::read_to_string(&self.path).context("read notes")?;
        Ok(serde_json::from_str(&raw).unwrap_or_default())
    }

    fn save(&self, store: &Store) -> Result<()> {
        let raw = serde_json::to_string_pretty(store)?;
        fs::write(&self.path, raw)?;
        Ok(())
    }

    pub fn list(&self) -> Result<Vec<Note>> {
        Ok(self.load()?.notes)
    }

    pub fn enqueue(
        &self,
        text: impl Into<String>,
        session_id: Option<String>,
        media_path: Option<String>,
        is_image: bool,
    ) -> Result<Note> {
        let mut store = self.load()?;
        let note = Note {
            id: Uuid::new_v4().to_string(),
            text: text.into(),
            session_id,
            created_at: chrono_lite_now(),
            media_path,
            is_image,
        };
        store.notes.push(note.clone());
        self.save(&store)?;
        Ok(note)
    }

    pub fn pop_next(&self) -> Result<Option<Note>> {
        let mut store = self.load()?;
        if store.notes.is_empty() {
            return Ok(None);
        }
        let note = store.notes.remove(0);
        self.save(&store)?;
        Ok(Some(note))
    }

    pub fn remove(&self, id: &str) -> Result<bool> {
        let mut store = self.load()?;
        let before = store.notes.len();
        store.notes.retain(|n| n.id != id);
        let removed = store.notes.len() != before;
        self.save(&store)?;
        Ok(removed)
    }

    pub fn clear(&self) -> Result<usize> {
        let mut store = self.load()?;
        let n = store.notes.len();
        store.notes.clear();
        self.save(&store)?;
        Ok(n)
    }

    pub fn count(&self) -> usize {
        self.load().map(|s| s.notes.len()).unwrap_or(0)
    }
}

fn chrono_lite_now() -> String {
    // RFC3339-ish without extra dep: unix secs is fine for queue display
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{secs}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enqueue_pop() {
        let tmp = tempfile::tempdir().unwrap();
        let q = NoteQueue::open(tmp.path()).unwrap();
        let n = q.enqueue("hello", Some("s1".into()), None, false).unwrap();
        assert_eq!(q.count(), 1);
        let p = q.pop_next().unwrap().unwrap();
        assert_eq!(p.id, n.id);
        assert_eq!(q.count(), 0);
    }
}
