//! Persistence for last-picked filter, recents, and per-filter
//! remembered argument values. Stored as JSON at
//! `~/Library/Application Support/gmic-affinity/settings.json`.
//!
//! Atomic-write via tmp + rename. On parse failure the broken file is
//! renamed to `settings.json.broken-<ts>` and defaults are returned;
//! never aborts a picker session.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::logging::log;

const SCHEMA_VERSION: u32 = 1;
const RECENTS_CAP: usize = 10;
const REMEMBERED_CAP: usize = 256;
const FILE_BYTES_CAP: u64 = 256 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Settings {
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default)]
    pub last: Option<LastChoice>,
    #[serde(default)]
    pub recent: Vec<RecentEntry>,
    #[serde(default)]
    pub remembered_args: BTreeMap<String, Vec<String>>,
}

fn default_version() -> u32 {
    0
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LastChoice {
    pub command: String,
    pub args: Vec<String>,
    pub display_path: String,
    pub ts: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RecentEntry {
    pub command: String,
    pub display_path: String,
    pub ts: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self::new()
    }
}

impl Settings {
    pub fn new() -> Self {
        Self {
            version: SCHEMA_VERSION,
            last: None,
            recent: Vec::new(),
            remembered_args: BTreeMap::new(),
        }
    }

    /// Read the settings file, returning defaults on absence or any
    /// parse / IO error. Renames a corrupted file aside so the user
    /// can recover it.
    pub fn load() -> Self {
        let path = match settings_path() {
            Some(p) => p,
            None => {
                log("settings: no HOME, using defaults");
                return Self::new();
            }
        };
        load_from(&path).unwrap_or_default()
    }

    /// Atomic-write: serialize → settings.json.tmp → fsync → rename().
    pub fn save(&self) {
        let Some(path) = settings_path() else {
            log("settings: no HOME, save skipped");
            return;
        };
        if let Some(parent) = path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                log(&format!("settings: create_dir_all({parent:?}) failed: {e}"));
                return;
            }
        }
        let serialized = match serde_json::to_vec_pretty(self) {
            Ok(b) => b,
            Err(e) => {
                log(&format!("settings: serialize failed: {e}"));
                return;
            }
        };
        if (serialized.len() as u64) > FILE_BYTES_CAP {
            log(&format!(
                "settings: serialized {} bytes exceeds cap {FILE_BYTES_CAP}, skipping save",
                serialized.len(),
            ));
            return;
        }
        let tmp = path.with_extension("json.tmp");
        if let Err(e) = write_atomic(&tmp, &path, &serialized) {
            log(&format!("settings: atomic save failed: {e}"));
        }
    }

    /// Record a user pick into `last`, push onto `recent` (dedup +
    /// cap), and update `remembered_args[command]` with the same args.
    pub fn record_pick(&mut self, command: &str, args: Vec<String>, display_path: &str, ts: &str) {
        self.version = SCHEMA_VERSION;
        self.last = Some(LastChoice {
            command: command.to_string(),
            args: args.clone(),
            display_path: display_path.to_string(),
            ts: ts.to_string(),
        });
        self.recent.retain(|r| r.command != command);
        self.recent.insert(
            0,
            RecentEntry {
                command: command.to_string(),
                display_path: display_path.to_string(),
                ts: ts.to_string(),
            },
        );
        if self.recent.len() > RECENTS_CAP {
            self.recent.truncate(RECENTS_CAP);
        }
        self.remembered_args.insert(command.to_string(), args);
        if self.remembered_args.len() > REMEMBERED_CAP {
            // BTreeMap has no LRU eviction; for v1 we shed the lexically
            // first entry that isn't the just-recorded one. Good enough
            // — REMEMBERED_CAP is 256 and won't be reached in practice.
            let evict = self
                .remembered_args
                .keys()
                .find(|k| k.as_str() != command)
                .cloned();
            if let Some(k) = evict {
                self.remembered_args.remove(&k);
            }
        }
    }

    /// Bump just `last.ts` for the Last-Filter case (where args don't change).
    pub fn touch_last(&mut self, ts: &str) {
        if let Some(last) = &mut self.last {
            last.ts = ts.to_string();
        }
    }
}

fn settings_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join("Library/Application Support/gmic-affinity/settings.json"))
}

fn load_from(path: &Path) -> Option<Settings> {
    let mut file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return None, // absence is normal
    };
    let mut buf = String::new();
    if let Err(e) = file.read_to_string(&mut buf) {
        log(&format!("settings: read failed: {e}; using defaults"));
        rename_broken(path);
        return None;
    }
    match serde_json::from_str::<Settings>(&buf) {
        Ok(mut s) => {
            if s.version > SCHEMA_VERSION {
                log(&format!(
                    "settings: file version {} > current {SCHEMA_VERSION}; using defaults without overwriting",
                    s.version,
                ));
                return None;
            }
            if s.version == 0 {
                s.version = SCHEMA_VERSION;
            }
            Some(s)
        }
        Err(e) => {
            log(&format!("settings: parse failed: {e}; renaming aside"));
            rename_broken(path);
            None
        }
    }
}

fn rename_broken(path: &Path) {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let broken = path.with_extension(format!("json.broken-{ts}"));
    if let Err(e) = std::fs::rename(path, &broken) {
        log(&format!("settings: rename to {broken:?} failed: {e}"));
    }
}

fn write_atomic(tmp: &Path, final_: &Path, bytes: &[u8]) -> std::io::Result<()> {
    {
        let mut f = std::fs::File::create(tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    std::fs::rename(tmp, final_)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn round_trip_empty() {
        let s = Settings::new();
        let json = serde_json::to_string(&s).unwrap();
        let back: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn record_pick_caps_recent_at_10() {
        let mut s = Settings::new();
        for i in 0..15 {
            s.record_pick(&format!("cmd{i}"), vec![], &format!("Path{i}"), "ts");
        }
        assert_eq!(s.recent.len(), 10);
        assert_eq!(s.recent[0].command, "cmd14"); // most recent first
    }

    #[test]
    fn record_pick_dedupes_recent() {
        let mut s = Settings::new();
        s.record_pick("a", vec![], "A", "1");
        s.record_pick("b", vec![], "B", "2");
        s.record_pick("a", vec![], "A", "3");
        assert_eq!(s.recent.len(), 2);
        assert_eq!(s.recent[0].command, "a");
        assert_eq!(s.recent[0].ts, "3");
    }

    #[test]
    fn record_pick_updates_remembered_args() {
        let mut s = Settings::new();
        s.record_pick("blur", vec!["3".into()], "X", "t");
        assert_eq!(s.remembered_args.get("blur").unwrap(), &vec!["3"]);
        s.record_pick("blur", vec!["7".into()], "X", "t2");
        assert_eq!(s.remembered_args.get("blur").unwrap(), &vec!["7"]);
    }

    #[test]
    fn atomic_write_and_read_round_trip() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let tmp = path.with_extension("json.tmp");
        let mut s = Settings::new();
        s.record_pick("blur", vec!["3".into()], "Effects/Blur", "ts");
        let bytes = serde_json::to_vec_pretty(&s).unwrap();
        write_atomic(&tmp, &path, &bytes).unwrap();
        assert!(!tmp.exists());
        let back: Settings =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn broken_file_returns_defaults_and_renames() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, "{not json}").unwrap();
        let result = load_from(&path);
        assert!(result.is_none());
        let mut found_broken = false;
        for entry in std::fs::read_dir(dir.path()).unwrap() {
            let name = entry.unwrap().file_name();
            if name.to_string_lossy().contains("broken") {
                found_broken = true;
            }
        }
        assert!(found_broken, "expected a *.broken-* file in tempdir");
    }

    #[test]
    fn future_version_returns_defaults_without_clobbering() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, r#"{"version":999}"#).unwrap();
        assert!(load_from(&path).is_none());
        // file should still be present unchanged
        assert!(path.exists());
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            r#"{"version":999}"#,
        );
    }

    #[test]
    fn missing_version_field_treats_as_v0_and_migrates() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, r#"{"recent":[]}"#).unwrap();
        let s = load_from(&path).unwrap();
        assert_eq!(s.version, SCHEMA_VERSION);
    }
}
