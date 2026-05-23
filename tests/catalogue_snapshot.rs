//! Smoke test: bundled gmic-catalogue.gmic.gz decompresses, parses,
//! and contains a sensible number of filters + known anchors.

use GmicFilter::catalogue::{self, Filter, Folder, Node, ParamKind};

#[test]
fn snapshot_decompresses_parses_and_has_minimum_content() {
    let cat = catalogue::builtin();
    let (folders, filters) = count(&cat.root);
    assert!(
        folders >= 15,
        "expected >=15 top-or-nested folders, got {folders}",
    );
    assert!(filters >= 400, "expected >=400 filters, got {filters}");
}

#[test]
fn anchor_filters_present() {
    let cat = catalogue::builtin();
    let mut commands: Vec<&str> = Vec::new();
    collect_commands(&cat.root, &mut commands);
    // Anchors chosen from the bundled snapshot (gmic 3.7.6). If a future
    // catalogue refresh removes these, pick the closest surviving filter
    // and update both names here.
    for anchor in ["fx_painting", "fx_glow"] {
        assert!(
            commands.contains(&anchor),
            "expected anchor command {anchor} in catalogue (sample: {:?})",
            &commands[..commands.len().min(5)],
        );
    }
}

#[test]
fn fx_linify_defaults_disable_preview_progression_for_headless_run() {
    let cat = catalogue::builtin();
    let linify = find_filter(&cat.root, "fx_linify").expect("fx_linify must stay in catalogue");
    assert_eq!(
        default_cli_args(linify),
        vec!["40", "2", "40", "10", "24", "0", "0"]
    );
}

fn count(folder: &Folder) -> (usize, usize) {
    let mut folders = 0;
    let mut filters = 0;
    for child in &folder.children {
        match child {
            Node::Folder(f) => {
                folders += 1;
                let (sf, sfi) = count(f);
                folders += sf;
                filters += sfi;
            }
            Node::Filter(_) => filters += 1,
        }
    }
    (folders, filters)
}

fn collect_commands<'a>(folder: &'a Folder, out: &mut Vec<&'a str>) {
    for child in &folder.children {
        match child {
            Node::Folder(f) => collect_commands(f, out),
            Node::Filter(f) => out.push(&f.command),
        }
    }
}

fn find_filter<'a>(folder: &'a Folder, command: &str) -> Option<&'a Filter> {
    for child in &folder.children {
        match child {
            Node::Folder(f) => {
                if let Some(found) = find_filter(f, command) {
                    return Some(found);
                }
            }
            Node::Filter(f) if f.command == command => return Some(f),
            Node::Filter(_) => {}
        }
    }
    None
}

fn default_cli_args(filter: &Filter) -> Vec<String> {
    filter
        .params
        .iter()
        .filter_map(|param| match &param.kind {
            ParamKind::Int { default, .. } => Some(default.to_string()),
            ParamKind::Float { default, .. } => Some(format_float(*default)),
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

fn format_float(v: f64) -> String {
    if (v.round() - v).abs() < 1e-9 {
        format!("{}", v.round() as i64)
    } else {
        let s = format!("{v:.4}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}
