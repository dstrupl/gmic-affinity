//! Decide whether a G'MIC filter should be hidden from the picker.
//!
//! A filter is excluded when it cannot run as a single-image,
//! non-interactive transform with default parameters — multi-input
//! (blend/compositing), interactive (needs live user input), or
//! external-data / non-filter utility entries. Two sources combine: a
//! committed baked list emitted from the preview generator's structural
//! skips, and conservative in-code heuristics as a backstop.

use std::collections::HashSet;
use std::sync::OnceLock;

/// Conservative static rules catching the structural classes by name,
/// as a backstop to the baked list (and the only source when previews
/// were never generated). Deliberately NOT exhaustive — the baked list
/// carries the long tail.
pub fn is_excluded_by_heuristic(command: &str) -> bool {
    // Interactive: filters that require live user input in gmic-qt.
    if command.ends_with("_interactive") {
        return true;
    }
    const INTERACTIVE: &[&str] = &["fx_MorphoPaint"];
    // Multi-input / compositing: need a second image we don't provide.
    const MULTI_INPUT: &[&str] = &[
        "fx_blend",
        "fx_blend_fade",
        "fx_clut_from_ab",
        "fx_transfer_pca",
        "fx_layer_cake",
        "fx_layer_cake_2",
    ];
    // External-data / non-filter utility entries.
    const EXTERNAL: &[&str] = &["gui_download_all_data"];
    INTERACTIVE.contains(&command) || MULTI_INPUT.contains(&command) || EXTERNAL.contains(&command)
}

/// The committed baked exclusion list, parsed once. Lines that are blank
/// or start with `#` are ignored; everything else is a gmic command.
pub fn baked_exclusions() -> &'static HashSet<String> {
    static BAKED: OnceLock<HashSet<String>> = OnceLock::new();
    BAKED.get_or_init(|| {
        const RAW: &str = include_str!("../../assets/excluded-filters.txt");
        RAW.lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(|l| l.to_string())
            .collect()
    })
}

/// True if `command` should be hidden from the picker.
pub fn is_excluded(command: &str) -> bool {
    baked_exclusions().contains(command) || is_excluded_by_heuristic(command)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heuristic_excludes_interactive_suffix() {
        assert!(is_excluded_by_heuristic("fx_curves_interactive"));
        assert!(is_excluded_by_heuristic("fx_morph_interactive"));
    }

    #[test]
    fn heuristic_excludes_known_multi_input() {
        assert!(is_excluded_by_heuristic("fx_blend"));
        assert!(is_excluded_by_heuristic("fx_clut_from_ab"));
    }

    #[test]
    fn heuristic_excludes_external_nonfilter() {
        assert!(is_excluded_by_heuristic("gui_download_all_data"));
    }

    #[test]
    fn heuristic_keeps_normal_filters() {
        assert!(!is_excluded_by_heuristic("fx_old_photo"));
        assert!(!is_excluded_by_heuristic("fx_blur"));
        assert!(!is_excluded_by_heuristic("fx_painting"));
    }

    #[test]
    fn baked_list_parses_ignoring_comments_and_blanks() {
        let set = baked_exclusions();
        // A known seeded command is present; comments/blank lines are not.
        assert!(set.contains("fx_blend"));
        assert!(!set.contains(""));
        assert!(!set.iter().any(|c| c.starts_with('#')));
    }

    #[test]
    fn is_excluded_combines_both_sources() {
        // Heuristic-only hit (suffix not necessarily in the baked file).
        assert!(is_excluded("fx_curves_interactive"));
        // Baked-list hit.
        assert!(is_excluded("fx_blend"));
        // Neither.
        assert!(!is_excluded("fx_old_photo"));
    }
}
