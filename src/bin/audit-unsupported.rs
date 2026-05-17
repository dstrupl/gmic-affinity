//! Walk the bundled catalogue, find every parameter the parser has
//! classified as [`ParamKind::Unknown`], group them by leading
//! function name (the bit before `(`), and print a frequency
//! histogram plus a small sample of distinct payloads per group.
//!
//! Run via: `cargo run --bin audit-unsupported`
//!
//! Output shape:
//!
//! ```text
//! count   kind                         sample
//! -----   ----                         ------
//!   612   point                        point(5,5,0,1,255,255,255,128,10)
//!   118   value                        value(0)
//!    23   color(#RRGGBBAA)             color(#000000ff)
//!     7   <bare>                       <bare>
//!     …
//! TOTAL: 781 unsupported params across 1493 filters (12.4%)
//! ```
//!
//! The catalogue picker hides the empty-label noise (G'MIC's hidden
//! `value(0)` placeholders etc.) but they still count here so we
//! see the real surface area when prioritising parser improvements.

use std::collections::BTreeMap;

use GmicFilter::catalogue::{self, Folder, Node, ParamKind};

#[derive(Default)]
struct Bucket {
    count: usize,
    sample: String,
}

fn main() {
    let cat = catalogue::builtin();
    let mut buckets: BTreeMap<String, Bucket> = BTreeMap::new();
    let mut total_filters: usize = 0;
    let mut total_unsupported: usize = 0;
    walk(
        &cat.root,
        &mut buckets,
        &mut total_filters,
        &mut total_unsupported,
    );

    // Sort by descending count for the printout — BTreeMap is sorted
    // alphabetically which buries the high-frequency entries.
    let mut rows: Vec<(&String, &Bucket)> = buckets.iter().collect();
    rows.sort_by(|a, b| b.1.count.cmp(&a.1.count).then(a.0.cmp(b.0)));

    println!("count   kind                         sample");
    println!("-----   ----                         ------");
    for (kind, bucket) in &rows {
        println!("{:>5}   {:<28} {}", bucket.count, kind, bucket.sample);
    }
    let pct = if total_filters == 0 {
        0.0
    } else {
        (total_unsupported as f64) * 100.0 / total_filter_param_count(cat)
    };
    println!(
        "\nTOTAL: {} unsupported params across {} filters ({:.1}% of all params)",
        total_unsupported, total_filters, pct,
    );
}

fn walk(
    folder: &Folder,
    buckets: &mut BTreeMap<String, Bucket>,
    total_filters: &mut usize,
    total_unsupported: &mut usize,
) {
    for child in &folder.children {
        match child {
            Node::Folder(f) => walk(f, buckets, total_filters, total_unsupported),
            Node::Filter(f) => {
                *total_filters += 1;
                for param in &f.params {
                    if let ParamKind::Unknown(raw) = &param.kind {
                        *total_unsupported += 1;
                        let kind = leading_kind(raw);
                        let entry = buckets.entry(kind).or_default();
                        entry.count += 1;
                        if entry.sample.is_empty() {
                            entry.sample = raw.clone();
                        }
                    }
                }
            }
        }
    }
}

/// Total parameter count across every filter, used only to compute
/// the unsupported percentage. Two passes over the catalogue is fine —
/// this is a one-shot dev tool.
fn total_filter_param_count(cat: &catalogue::Catalogue) -> f64 {
    fn walk(folder: &Folder, total: &mut usize) {
        for child in &folder.children {
            match child {
                Node::Folder(f) => walk(f, total),
                Node::Filter(f) => *total += f.params.len(),
            }
        }
    }
    let mut total = 0;
    walk(&cat.root, &mut total);
    total as f64
}

/// Extract the leading function name from an Unknown payload:
///
/// - `point(5,5,...)`       -> `"point"`
/// - `color(#000000ff)`     -> `"color(#RRGGBBAA)"`  (special-case so
///   the hex form sorts separately from the comma-separated form)
/// - `value(0)`             -> `"value"`
/// - `~point(...)`          -> `"~point"`
/// - free-form text, no `(` -> `"<bare>"`
fn leading_kind(raw: &str) -> String {
    let trimmed = raw.trim();
    let Some(open) = trimmed.find('(') else {
        return "<bare>".to_string();
    };
    let head = trimmed[..open].trim().to_string();
    // Distinguish color(#hex) from color(r,g,b) — they share a head
    // but our parse_color only accepts the r,g,b form today.
    if head == "color" {
        let inner = &trimmed[open + 1..];
        if inner.starts_with('#') {
            return "color(#RRGGBBAA)".to_string();
        }
    }
    head
}
