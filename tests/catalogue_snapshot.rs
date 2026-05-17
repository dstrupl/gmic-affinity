//! Smoke test: bundled gmic-catalogue.gmic.gz decompresses, parses,
//! and contains a sensible number of filters + known anchors.

use GmicFilter::catalogue::{self, Folder, Node};

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
