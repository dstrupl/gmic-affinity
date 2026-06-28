//! Build-time preview-generation support, shared between the
//! `gen-previews` binary and the live picker UI.
//!
//! Everything here compiles in the default (non-`live`) build: it is
//! pure Rust with no AppKit dependency. The binary uses the whole
//! module; the UI uses only [`sanitise_key`] to re-derive a filter's
//! preview filename without parsing the manifest at runtime.

use sha2::{Digest, Sha256};

pub mod manifest;

/// Map a gmic command to a stable, filesystem-safe filename stem.
///
/// The visible portion keeps `[A-Za-z0-9_]` from the command (other
/// bytes become `_`) so files are recognisable; a short hex suffix of
/// the *original* command guarantees two distinct commands never map
/// to the same key even when their sanitised prefixes collide.
pub fn sanitise_key(command: &str) -> String {
    let safe: String = command
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let digest = Sha256::digest(command.as_bytes());
    let suffix = &hex8(&digest);
    format!("{safe}-{suffix}")
}

fn hex8(bytes: &[u8]) -> String {
    bytes.iter().take(4).map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod sanitise_tests {
    use super::sanitise_key;

    #[test]
    fn keeps_safe_chars() {
        // Plain fx names survive except for the disambiguating suffix.
        let k = sanitise_key("fx_oldphoto");
        assert!(k.starts_with("fx_oldphoto-"));
        assert!(k
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'));
    }

    #[test]
    fn replaces_unsafe_chars() {
        let k = sanitise_key("foo/bar baz.qux");
        // No path separators, spaces, or dots in the safe portion.
        let safe = k.rsplit_once('-').unwrap().0;
        assert!(safe.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'));
    }

    #[test]
    fn distinct_commands_get_distinct_keys() {
        // Same sanitised prefix but different originals must differ.
        assert_ne!(sanitise_key("a/b"), sanitise_key("a_b"));
    }

    #[test]
    fn stable_for_same_input() {
        assert_eq!(sanitise_key("fx_painting"), sanitise_key("fx_painting"));
    }
}
