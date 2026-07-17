//! Browse `~/.grok/sessions` for phone session picker.

use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Debug, Clone)]
pub struct SessionEntry {
    pub session_id: String,
    pub cwd: String,
    pub title: String,
    pub updated: SystemTime,
    pub disk_path: PathBuf,
}

#[derive(Debug, Deserialize)]
struct SummaryFile {
    info: Option<SummaryInfo>,
    session_summary: Option<String>,
    generated_title: Option<String>,
    updated_at: Option<String>,
    last_active_at: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SummaryInfo {
    id: Option<String>,
    cwd: Option<String>,
}

/// List recent sessions (newest first), capped.
pub fn list_sessions(sessions_root: &Path, limit: usize) -> Vec<SessionEntry> {
    let mut out = Vec::new();
    let Ok(groups) = fs::read_dir(sessions_root) else {
        return out;
    };
    for group in groups.flatten() {
        let gpath = group.path();
        if !gpath.is_dir() {
            continue;
        }
        // group name is URL-encoded cwd or bare
        let group_cwd = decode_cwd_group(group.file_name().to_string_lossy().as_ref());
        let Ok(sess_dirs) = fs::read_dir(&gpath) else {
            continue;
        };
        for sdir in sess_dirs.flatten() {
            let sp = sdir.path();
            if !sp.is_dir() {
                continue;
            }
            let name = sdir.file_name().to_string_lossy().to_string();
            // UUID-ish session folders
            if name.len() < 8 || !name.contains('-') {
                continue;
            }
            let summary_path = sp.join("summary.json");
            let mut title = name.chars().take(12).collect::<String>();
            let mut cwd = group_cwd.clone();
            let mut sid = name.clone();
            if let Ok(raw) = fs::read_to_string(&summary_path) {
                if let Ok(sum) = serde_json::from_str::<SummaryFile>(&raw) {
                    if let Some(info) = sum.info {
                        if let Some(id) = info.id {
                            sid = id;
                        }
                        if let Some(c) = info.cwd {
                            cwd = c;
                        }
                    }
                    title = sum
                        .generated_title
                        .or(sum.session_summary)
                        .filter(|t| !t.is_empty())
                        .unwrap_or(title);
                }
            }
            let updated = fs::metadata(&sp)
                .and_then(|m| m.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            out.push(SessionEntry {
                session_id: sid,
                cwd,
                title,
                updated,
                disk_path: sp,
            });
        }
    }
    out.sort_by(|a, b| b.updated.cmp(&a.updated));
    out.truncate(limit);
    out
}

fn decode_cwd_group(name: &str) -> String {
    // percent-decode lightly
    let mut s = String::new();
    let b = name.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let hex = std::str::from_utf8(&b[i + 1..i + 3]).ok();
            if let Some(h) = hex {
                if let Ok(v) = u8::from_str_radix(h, 16) {
                    s.push(v as char);
                    i += 3;
                    continue;
                }
            }
        }
        s.push(b[i] as char);
        i += 1;
    }
    if s.starts_with('/') {
        s
    } else {
        format!("/{s}")
    }
}

/// Last N user/assistant lines from chat_history.jsonl (best-effort).
pub fn peek_history(session_dir: &Path, n: usize) -> String {
    let path = session_dir.join("chat_history.jsonl");
    let Ok(raw) = fs::read_to_string(&path) else {
        return "(no chat_history.jsonl)".into();
    };
    let mut lines: Vec<String> = Vec::new();
    for line in raw.lines().rev() {
        if lines.len() >= n {
            break;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let role = v
            .get("role")
            .or_else(|| v.pointer("/message/role"))
            .and_then(|r| r.as_str())
            .unwrap_or("?");
        let text = v
            .get("text")
            .or_else(|| v.pointer("/message/content/0/text"))
            .or_else(|| v.get("content"))
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .chars()
            .take(200)
            .collect::<String>();
        if text.is_empty() {
            continue;
        }
        lines.push(format!("[{}] {}", role.to_uppercase(), text));
    }
    lines.reverse();
    if lines.is_empty() {
        "(empty history)".into()
    } else {
        lines.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;

    #[test]
    fn list_and_decode() {
        let tmp = tempfile::tempdir().unwrap();
        let group = tmp.path().join("%2Ftmp%2Fproj");
        let sid = "019f6cbf-cca8-7cc1-92ef-b2f2b7d354f6";
        let sdir = group.join(sid);
        fs::create_dir_all(&sdir).unwrap();
        let mut f = fs::File::create(sdir.join("summary.json")).unwrap();
        write!(
            f,
            r#"{{"info":{{"id":"{sid}","cwd":"/tmp/proj"}},"generated_title":"Hello World"}}"#
        )
        .unwrap();
        let list = list_sessions(tmp.path(), 10);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].session_id, sid);
        assert_eq!(list[0].cwd, "/tmp/proj");
        assert_eq!(list[0].title, "Hello World");
    }
}
