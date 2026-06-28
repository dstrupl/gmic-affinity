//! Reconcile a saved `remembered_args` Vec<String> against the
//! current filter's parameter list. Out-of-band drift between gmic
//! releases means we can't assume the saved vector lines up; this
//! module picks per-row whether to keep the saved value or fall back
//! to the parameter's default.

use crate::catalogue::{Param, ParamKind};

pub fn reconcile(remembered: &[String], params: &[Param]) -> Vec<String> {
    params
        .iter()
        .enumerate()
        .map(|(i, p)| {
            remembered
                .get(i)
                .filter(|v| value_matches_kind(v, &p.kind))
                .cloned()
                .unwrap_or_else(|| default_for(&p.kind))
        })
        .collect()
}

fn value_matches_kind(value: &str, kind: &ParamKind) -> bool {
    match kind {
        ParamKind::Int { min, max, .. } => value
            .parse::<i64>()
            .map(|v| v >= *min && v <= *max)
            .unwrap_or(false),
        ParamKind::Float { min, max, .. } => value
            .parse::<f64>()
            .map(|v| v >= *min && v <= *max)
            .unwrap_or(false),
        ParamKind::Bool { .. } => matches!(value, "true" | "false" | "0" | "1"),
        ParamKind::Choice { choices, .. } => choices.iter().any(|c| c == value),
        ParamKind::Color { .. } => {
            // COMMIT 2: accept 3 OR 4 comma-separated u8 parts (RGB or RGBA)
            let count = value.split(',').count();
            (count == 3 || count == 4) && value.split(',').all(|c| c.trim().parse::<u8>().is_ok())
        }
        ParamKind::Text { .. } => true,
        ParamKind::Note(_) | ParamKind::Separator | ParamKind::Link { .. } => true,
        // Internal params are non-interactive. Never trust stale
        // remembered values here; always fall back to the declaration
        // default so hidden gmic-qt controls stay in the headless-safe
        // state chosen by the parser.
        ParamKind::Internal { .. } => false,
        ParamKind::Unknown(_) => true,
    }
}

fn default_for(kind: &ParamKind) -> String {
    match kind {
        ParamKind::Int { default, .. } => default.to_string(),
        ParamKind::Float { default, .. } => default.to_string(),
        ParamKind::Bool { default } => default.to_string(),
        ParamKind::Choice { choices, default } => {
            choices.get(*default).cloned().unwrap_or_default()
        }
        ParamKind::Color { default_rgba } => {
            format!(
                "{},{},{},{}",
                default_rgba[0], default_rgba[1], default_rgba[2], default_rgba[3]
            )
        }
        ParamKind::Text { default } => default.clone(),
        ParamKind::Note(_) | ParamKind::Separator | ParamKind::Link { .. } => String::new(),
        ParamKind::Internal { default, .. } => default.clone(),
        ParamKind::Unknown(s) => s.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalogue::{Param, ParamKind};

    fn p(label: &str, kind: ParamKind) -> Param {
        Param {
            label: label.into(),
            kind,
        }
    }

    #[test]
    fn matching_int_is_kept() {
        let params = vec![p(
            "R",
            ParamKind::Int {
                default: 5,
                min: 0,
                max: 10,
            },
        )];
        let out = reconcile(&["7".into()], &params);
        assert_eq!(out, vec!["7".to_string()]);
    }

    #[test]
    fn out_of_range_int_falls_back() {
        let params = vec![p(
            "R",
            ParamKind::Int {
                default: 5,
                min: 0,
                max: 10,
            },
        )];
        let out = reconcile(&["99".into()], &params);
        assert_eq!(out, vec!["5".to_string()]);
    }

    #[test]
    fn extra_saved_values_are_ignored() {
        let params = vec![p(
            "R",
            ParamKind::Int {
                default: 1,
                min: 0,
                max: 10,
            },
        )];
        let out = reconcile(&["3".into(), "4".into(), "5".into()], &params);
        assert_eq!(out, vec!["3".to_string()]);
    }

    #[test]
    fn missing_saved_values_use_defaults() {
        let params = vec![
            p(
                "A",
                ParamKind::Int {
                    default: 1,
                    min: 0,
                    max: 10,
                },
            ),
            p(
                "B",
                ParamKind::Float {
                    default: 0.5,
                    min: 0.0,
                    max: 1.0,
                },
            ),
        ];
        let out = reconcile(&["7".into()], &params);
        assert_eq!(out, vec!["7".to_string(), "0.5".to_string()]);
    }

    #[test]
    fn type_mismatch_uses_default() {
        let params = vec![p(
            "R",
            ParamKind::Int {
                default: 5,
                min: 0,
                max: 10,
            },
        )];
        let out = reconcile(&["not-a-number".into()], &params);
        assert_eq!(out, vec!["5".to_string()]);
    }

    #[test]
    fn internal_saved_value_is_overwritten() {
        let params = vec![p(
            "headless-preview",
            ParamKind::Internal {
                label: "headless-preview".into(),
                default: "0".into(),
            },
        )];
        let out = reconcile(&["1".into()], &params);
        assert_eq!(out, vec!["0".to_string()]);
    }
}
