//! Preview manifest: per-filter up-to-date tracking shared by the
//! generator. Pure logic — no gmic, no filesystem beyond load/save.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Manifest {
    #[serde(default)]
    pub source_hash: String,
    #[serde(default)]
    pub gmic_version: String,
    #[serde(default)]
    pub entries: BTreeMap<String, Entry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub input_hash: String,
    pub status: EntryStatus,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EntryStatus {
    Ok,
    Skip,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Recompute,
    Keep,
}

/// Hash exactly the inputs that change a rendered preview. A change in
/// any of them flips the hash and triggers a recompute; catalogue
/// folder/display-name moves are deliberately NOT included because they
/// don't alter the rendered pixels.
pub fn input_hash(source_hash: &str, gmic_version: &str, command: &str, args: &[String]) -> String {
    let mut h = Sha256::new();
    // Length-prefix each field so concatenation can't alias across
    // boundaries (e.g. "ab"+"c" vs "a"+"bc").
    for field in [source_hash, gmic_version, command] {
        h.update((field.len() as u64).to_le_bytes());
        h.update(field.as_bytes());
    }
    h.update((args.len() as u64).to_le_bytes());
    for a in args {
        h.update((a.len() as u64).to_le_bytes());
        h.update(a.as_bytes());
    }
    format!("sha256:{:x}", h.finalize())
}

/// Decide whether a filter's preview must be regenerated.
pub fn decide(entry: Option<&Entry>, computed_hash: &str, file_exists: bool) -> Action {
    match entry {
        None => Action::Recompute,
        Some(e) if e.input_hash != computed_hash => Action::Recompute,
        // A recorded success whose PNG vanished must be re-rendered.
        Some(e) if e.status == EntryStatus::Ok && !file_exists => Action::Recompute,
        Some(_) => Action::Keep,
    }
}

/// Load a manifest, treating a missing or unparseable file as empty so
/// the next build simply regenerates everything.
pub fn load(path: &Path) -> Manifest {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
        Err(_) => Manifest::default(),
    }
}

/// Write the manifest as pretty JSON (stable key order via `BTreeMap`).
pub fn save(path: &Path, m: &Manifest) -> std::io::Result<()> {
    let json = serde_json::to_string_pretty(m).expect("manifest serialises");
    std::fs::write(path, json)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok_entry(hash: &str) -> Entry {
        Entry {
            input_hash: hash.into(),
            status: EntryStatus::Ok,
            file: Some("f.png".into()),
            reason: None,
        }
    }

    #[test]
    fn hash_changes_with_each_input() {
        let base = input_hash("s", "v", "cmd", &["1".into()]);
        assert_ne!(base, input_hash("s2", "v", "cmd", &["1".into()]));
        assert_ne!(base, input_hash("s", "v2", "cmd", &["1".into()]));
        assert_ne!(base, input_hash("s", "v", "cmd2", &["1".into()]));
        assert_ne!(base, input_hash("s", "v", "cmd", &["2".into()]));
    }

    #[test]
    fn hash_is_stable() {
        assert_eq!(
            input_hash("s", "v", "cmd", &["1".into(), "2".into()]),
            input_hash("s", "v", "cmd", &["1".into(), "2".into()]),
        );
    }

    #[test]
    fn missing_entry_recomputes() {
        assert_eq!(decide(None, "h", false), Action::Recompute);
    }

    #[test]
    fn hash_drift_recomputes() {
        let e = ok_entry("old");
        assert_eq!(decide(Some(&e), "new", true), Action::Recompute);
    }

    #[test]
    fn ok_but_file_gone_recomputes() {
        let e = ok_entry("h");
        assert_eq!(decide(Some(&e), "h", false), Action::Recompute);
    }

    #[test]
    fn fresh_ok_is_kept() {
        let e = ok_entry("h");
        assert_eq!(decide(Some(&e), "h", true), Action::Keep);
    }

    #[test]
    fn fresh_skip_is_kept() {
        let e = Entry {
            input_hash: "h".into(),
            status: EntryStatus::Skip,
            file: None,
            reason: Some("boom".into()),
        };
        // Skip entries have no file; matching hash keeps them.
        assert_eq!(decide(Some(&e), "h", false), Action::Keep);
    }

    #[test]
    fn load_missing_returns_default() {
        let m = load(Path::new("/no/such/manifest.json"));
        assert!(m.entries.is_empty());
    }
}
