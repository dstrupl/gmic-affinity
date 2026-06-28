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

/// Build a param-aligned vector of default values (one entry per param
/// in order). Silent params (Note/Separator/Link/Unknown) become `""`.
/// This is what `ChosenFilter.args` carries and what `reconcile` expects.
pub fn param_aligned_defaults(params: &[Param]) -> Vec<String> {
    params
        .iter()
        .map(|param| match &param.kind {
            ParamKind::Int { default, min, max } => (*default).clamp(*min, *max).to_string(),
            ParamKind::Float { default, min, max } => format_float(default.clamp(*min, *max)),
            ParamKind::Bool { default } => if *default { "1" } else { "0" }.to_string(),
            ParamKind::Choice { default, .. } => default.to_string(),
            ParamKind::Color { default_rgba } => {
                format!(
                    "{},{},{},{}",
                    default_rgba[0], default_rgba[1], default_rgba[2], default_rgba[3]
                )
            }
            ParamKind::Text { default } => default.clone(),
            ParamKind::Internal { default, .. } => default.clone(),
            ParamKind::Note(_)
            | ParamKind::Separator
            | ParamKind::Link { .. }
            | ParamKind::Unknown(_) => String::new(),
        })
        .collect()
}

/// Convert param-aligned values into the argv that gmic expects. Drops
/// silent params (Note/Separator/Link/Unknown); expands Color values
/// from one `"r,g,b,a"` token into 4 separate argv entries; passes other
/// value params through unchanged.
pub fn values_to_argv(params: &[Param], values: &[String]) -> Vec<String> {
    let mut argv = Vec::new();
    for (param, value) in params.iter().zip(values.iter()) {
        match &param.kind {
            ParamKind::Note(_)
            | ParamKind::Separator
            | ParamKind::Link { .. }
            | ParamKind::Unknown(_) => {
                // Silent params contribute nothing
            }
            ParamKind::Color { .. } => {
                // COMMIT 2: expand "r,g,b,a" into 4 separate argv entries
                for channel in value.split(',') {
                    argv.push(channel.to_string());
                }
            }
            _ => {
                argv.push(value.clone());
            }
        }
    }
    argv
}

/// Build the argv the picker would send to gmic for `params` with
/// every control left at its default. This MUST stay in lock-step with
/// `FormController::collect_values` in `src/ui/picker_form.rs`: gmic
/// receives a `Choice` as its selected *index*, a `Bool` as `"1"`/`"0"`,
/// and presentation-only params (`Note`/`Separator`/`Link`/`Unknown`)
/// contribute no argv entry at all.
pub fn default_argv(params: &[Param]) -> Vec<String> {
    values_to_argv(params, &param_aligned_defaults(params))
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
    fn color_expands_to_four_channels() {
        // COMMIT 2: Color expands to 4 separate argv entries
        let out = default_argv(&[p(ParamKind::Color {
            default_rgba: [10, 20, 30, 40],
        })]);
        assert_eq!(out, vec!["10", "20", "30", "40"]);
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

#[cfg(test)]
mod commit1_tests {
    use super::{default_argv, param_aligned_defaults, values_to_argv};
    use crate::catalogue::{Param, ParamKind};

    fn p(kind: ParamKind) -> Param {
        Param {
            label: "x".into(),
            kind,
        }
    }

    #[test]
    fn param_aligned_defaults_produces_one_entry_per_param() {
        let params = vec![
            p(ParamKind::Int {
                default: 5,
                min: 0,
                max: 10,
            }),
            p(ParamKind::Note("info".into())),
            p(ParamKind::Bool { default: true }),
            p(ParamKind::Separator),
            p(ParamKind::Float {
                default: 2.5,
                min: 0.0,
                max: 10.0,
            }),
        ];
        let defaults = param_aligned_defaults(&params);
        assert_eq!(defaults.len(), params.len());
        assert_eq!(defaults, vec!["5", "", "1", "", "2.5"]);
    }

    #[test]
    fn values_to_argv_drops_silent_params() {
        let params = vec![
            p(ParamKind::Int {
                default: 10,
                min: 0,
                max: 100,
            }),
            p(ParamKind::Note("skip".into())),
            p(ParamKind::Bool { default: false }),
            p(ParamKind::Separator),
            p(ParamKind::Text {
                default: "text".into(),
            }),
            p(ParamKind::Link {
                label: "L".into(),
                url: "u".into(),
            }),
        ];
        let values = vec![
            "42".into(),
            "".into(),
            "1".into(),
            "".into(),
            "hello".into(),
            "".into(),
        ];
        let argv = values_to_argv(&params, &values);
        // Only Int, Bool, Text should survive
        assert_eq!(argv, vec!["42", "1", "hello"]);
    }

    #[test]
    fn values_to_argv_keeps_order() {
        let params = vec![
            p(ParamKind::Int {
                default: 0,
                min: 0,
                max: 10,
            }),
            p(ParamKind::Float {
                default: 0.0,
                min: 0.0,
                max: 1.0,
            }),
            p(ParamKind::Bool { default: false }),
            p(ParamKind::Text { default: "".into() }),
        ];
        let values = vec!["7".into(), "0.5".into(), "1".into(), "foo".into()];
        let argv = values_to_argv(&params, &values);
        assert_eq!(argv, vec!["7", "0.5", "1", "foo"]);
    }

    #[test]
    fn default_argv_unchanged_for_no_silent_params() {
        // A filter with no Note/Separator/Link/Unknown should produce
        // the same output as before (via the new param_aligned→argv path)
        let params = vec![
            p(ParamKind::Int {
                default: 3,
                min: 0,
                max: 10,
            }),
            p(ParamKind::Bool { default: true }),
            p(ParamKind::Choice {
                choices: vec!["A".into(), "B".into()],
                default: 1,
            }),
        ];
        let argv = default_argv(&params);
        assert_eq!(argv, vec!["3", "1", "1"]);
    }

    #[test]
    fn default_argv_drops_silent_params() {
        let params = vec![
            p(ParamKind::Note("intro".into())),
            p(ParamKind::Int {
                default: 5,
                min: 0,
                max: 10,
            }),
            p(ParamKind::Separator),
            p(ParamKind::Bool { default: false }),
        ];
        let argv = default_argv(&params);
        // Only Int and Bool
        assert_eq!(argv, vec!["5", "0"]);
    }

    #[test]
    fn round_trip_param_aligned_defaults_through_reconcile_aligned() {
        use crate::catalogue::reconcile::reconcile;

        let params = vec![
            p(ParamKind::Int {
                default: 10,
                min: 0,
                max: 100,
            }),
            p(ParamKind::Note("skip".into())),
            p(ParamKind::Bool { default: true }),
            p(ParamKind::Separator),
            p(ParamKind::Text {
                default: "text".into(),
            }),
        ];

        let defaults = param_aligned_defaults(&params);
        let reconciled = reconcile(&defaults, &params);

        // Reconcile should preserve param-aligned structure
        assert_eq!(reconciled.len(), params.len());
        assert_eq!(reconciled, vec!["10", "", "1", "", "text"]);
    }
}

#[cfg(test)]
mod commit2_tests {
    use super::{param_aligned_defaults, values_to_argv};
    use crate::catalogue::{Param, ParamKind};

    fn p(kind: ParamKind) -> Param {
        Param {
            label: "x".into(),
            kind,
        }
    }

    #[test]
    fn color_value_expands_to_four_argv_entries() {
        // COMMIT 2: Color "r,g,b,a" expands to 4 separate argv entries
        let params = vec![
            p(ParamKind::Int {
                default: 1,
                min: 0,
                max: 10,
            }),
            p(ParamKind::Color {
                default_rgba: [100, 150, 200, 250],
            }),
            p(ParamKind::Bool { default: true }),
        ];
        let values = vec!["5".into(), "10,20,30,40".into(), "1".into()];
        let argv = values_to_argv(&params, &values);
        // Int + 4 color channels + Bool = 6 entries
        assert_eq!(argv, vec!["5", "10", "20", "30", "40", "1"]);
    }

    #[test]
    fn param_aligned_defaults_color_is_four_values() {
        // Color default in param-aligned form is "r,g,b,a"
        let params = vec![p(ParamKind::Color {
            default_rgba: [10, 20, 30, 40],
        })];
        let defaults = param_aligned_defaults(&params);
        assert_eq!(defaults, vec!["10,20,30,40"]);
    }

    #[test]
    fn multiple_colors_each_expand_to_four() {
        let params = vec![
            p(ParamKind::Color {
                default_rgba: [1, 2, 3, 4],
            }),
            p(ParamKind::Int {
                default: 99,
                min: 0,
                max: 100,
            }),
            p(ParamKind::Color {
                default_rgba: [5, 6, 7, 8],
            }),
        ];
        let values = vec!["11,12,13,14".into(), "50".into(), "21,22,23,24".into()];
        let argv = values_to_argv(&params, &values);
        // First color (4) + Int (1) + Second color (4) = 9 entries
        assert_eq!(
            argv,
            vec!["11", "12", "13", "14", "50", "21", "22", "23", "24"]
        );
    }

    #[test]
    fn argv_length_grows_by_three_per_color() {
        // A filter with N colors increases argv by 3N compared to no expansion
        // (1 param-aligned token → 4 argv entries, net +3)
        let params = vec![
            p(ParamKind::Int {
                default: 1,
                min: 0,
                max: 10,
            }),
            p(ParamKind::Color {
                default_rgba: [0, 0, 0, 0],
            }),
            p(ParamKind::Bool { default: false }),
        ];
        let defaults = param_aligned_defaults(&params);
        let argv = values_to_argv(&params, &defaults);
        // Int (1) + Color (4) + Bool (1) = 6
        assert_eq!(argv.len(), 6);
    }
}
