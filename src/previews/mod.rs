//! Build-time preview-generation support, shared between the
//! `gen-previews` binary and the live picker UI.
//!
//! Everything here compiles in the default (non-`live`) build: it is
//! pure Rust with no AppKit dependency. The binary uses the whole
//! module; the UI uses only [`sanitise_key`] to re-derive a filter's
//! preview filename without parsing the manifest at runtime.

use sha2::{Digest, Sha256};

use crate::catalogue::{Param, ParamKind};

pub mod manifest;

/// Format a float the way the picker's slider readback does: integers
/// print without a decimal point, fractions keep up to 4 places with
/// trailing zeros trimmed. Kept identical to the picker so previews
/// match the argv a real OK click would send.
pub fn format_float(v: f64) -> String {
    if (v.round() - v).abs() < 1e-9 {
        format!("{}", v.round() as i64)
    } else {
        let s = format!("{v:.4}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

/// Build the argv the picker would send to gmic for `params` with
/// every control left at its default. This MUST stay in lock-step with
/// `FormController::collect_values` in `src/ui/picker_form.rs`: gmic
/// receives a `Choice` as its selected *index*, a `Bool` as `"1"`/`"0"`,
/// and presentation-only params (`Note`/`Separator`/`Link`/`Unknown`)
/// contribute no argv entry at all.
pub fn default_argv(params: &[Param]) -> Vec<String> {
    params
        .iter()
        .filter_map(|param| match &param.kind {
            ParamKind::Int { default, min, max } => Some(default.clamp(min, max).to_string()),
            ParamKind::Float { default, min, max } => Some(format_float(default.clamp(*min, *max))),
            ParamKind::Bool { default } => Some(if *default { "1" } else { "0" }.to_string()),
            ParamKind::Choice { default, .. } => Some(default.to_string()),
            ParamKind::Color { default_rgb } => Some(format!(
                "{},{},{}",
                default_rgb[0], default_rgb[1], default_rgb[2]
            )),
            ParamKind::Text { default } => Some(default.clone()),
            ParamKind::Internal { default, .. } => Some(default.clone()),
            ParamKind::Note(_)
            | ParamKind::Separator
            | ParamKind::Link { .. }
            | ParamKind::Unknown(_) => None,
        })
        .collect()
}

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

#[cfg(test)]
mod argv_tests {
    use super::{default_argv, format_float};
    use crate::catalogue::{Param, ParamKind};

    fn p(kind: ParamKind) -> Param {
        Param {
            label: "x".into(),
            kind,
        }
    }

    #[test]
    fn format_float_trims_zeros() {
        assert_eq!(format_float(0.5), "0.5");
        assert_eq!(format_float(2.0), "2");
        assert_eq!(format_float(1.2500), "1.25");
    }

    #[test]
    fn int_is_clamped_default() {
        let out = default_argv(&[p(ParamKind::Int {
            default: 5,
            min: 0,
            max: 10,
        })]);
        assert_eq!(out, vec!["5"]);
    }

    #[test]
    fn bool_is_one_or_zero() {
        let out = default_argv(&[
            p(ParamKind::Bool { default: true }),
            p(ParamKind::Bool { default: false }),
        ]);
        assert_eq!(out, vec!["1", "0"]);
    }

    #[test]
    fn choice_is_index_not_text() {
        let out = default_argv(&[p(ParamKind::Choice {
            choices: vec!["A".into(), "B".into(), "C".into()],
            default: 2,
        })]);
        assert_eq!(out, vec!["2"]);
    }

    #[test]
    fn color_is_rgb_bytes() {
        let out = default_argv(&[p(ParamKind::Color {
            default_rgb: [10, 20, 30],
        })]);
        assert_eq!(out, vec!["10,20,30"]);
    }

    #[test]
    fn presentation_params_contribute_nothing() {
        let out = default_argv(&[
            p(ParamKind::Note("hi".into())),
            p(ParamKind::Separator),
            p(ParamKind::Link {
                label: "L".into(),
                url: "u".into(),
            }),
            p(ParamKind::Unknown("point(1,2)".into())),
        ]);
        assert!(out.is_empty());
    }

    #[test]
    fn internal_contributes_default_verbatim() {
        let out = default_argv(&[p(ParamKind::Internal {
            label: "hidden".into(),
            default: "0".into(),
        })]);
        assert_eq!(out, vec!["0"]);
    }

    #[test]
    fn text_is_verbatim() {
        let out = default_argv(&[p(ParamKind::Text {
            default: "hello world".into(),
        })]);
        assert_eq!(out, vec!["hello world"]);
    }
}
