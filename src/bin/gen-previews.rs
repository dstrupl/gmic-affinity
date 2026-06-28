//! Build-time preview generator. See docs/superpowers/plans for design.
//!
//! Walks the bundled catalogue, renders one PNG per renderable filter
//! against a single sample image, and records results in a manifest so
//! reruns only recompute filters whose inputs changed.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use GmicFilter::catalogue::{self, Filter, Folder, Node};
use GmicFilter::gmic::{self, GmicError};
use GmicFilter::previews::manifest::{self, Action, Entry, EntryStatus, Manifest};
use GmicFilter::previews::{default_argv, sanitise_key};

/// A unit of work: one renderable filter plus the argv we will send.
struct Job {
    command: String,
    args: Vec<String>,
    key: String,
}

/// Flatten the catalogue tree into render jobs, deduplicating by command.
/// Some filters appear in multiple folders; we only render each command once.
fn collect_jobs(folder: &Folder, out: &mut Vec<Job>) {
    let mut seen = std::collections::HashSet::new();
    collect_jobs_dedup(folder, out, &mut seen);
}

fn collect_jobs_dedup(
    folder: &Folder,
    out: &mut Vec<Job>,
    seen: &mut std::collections::HashSet<String>,
) {
    for child in &folder.children {
        match child {
            Node::Folder(f) => collect_jobs_dedup(f, out, seen),
            Node::Filter(Filter {
                command, params, ..
            }) => {
                if seen.insert(command.clone()) {
                    out.push(Job {
                        command: command.clone(),
                        args: default_argv(params),
                        key: sanitise_key(command),
                    });
                }
            }
        }
    }
}

struct Config {
    source: PathBuf,
    out: PathBuf,
    manifest: PathBuf,
    jobs: usize,
    only: Option<String>,
    force: bool,
}

fn parse_args() -> Config {
    let mut cfg = Config {
        source: PathBuf::from("assets/preview-source.tiff"),
        out: PathBuf::from("previews"),
        manifest: PathBuf::from("previews/manifest.json"),
        jobs: std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4),
        only: None,
        force: false,
    };
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        match flag.as_str() {
            "--source" => cfg.source = it.next().expect("--source needs a value").into(),
            "--out" => cfg.out = it.next().expect("--out needs a value").into(),
            "--manifest" => cfg.manifest = it.next().expect("--manifest needs a value").into(),
            "--jobs" => cfg.jobs = it.next().and_then(|v| v.parse().ok()).unwrap_or(cfg.jobs),
            "--only" => cfg.only = Some(it.next().expect("--only needs a value")),
            "--force" => cfg.force = true,
            other => panic!("unknown flag: {other}"),
        }
    }
    cfg
}

fn sha256_file(path: &Path) -> std::io::Result<String> {
    use sha2::{Digest, Sha256};
    let bytes = std::fs::read(path)?;
    Ok(format!("sha256:{:x}", Sha256::digest(&bytes)))
}

fn gmic_version(gmic: &Path) -> String {
    let out = std::process::Command::new(gmic).arg("--version").output();
    match out {
        Ok(o) => String::from_utf8_lossy(&o.stdout).trim().to_string(),
        Err(_) => "unknown".to_string(),
    }
}

fn main() {
    let cfg = parse_args();
    std::fs::create_dir_all(&cfg.out).expect("create out dir");

    let gmic = gmic::locate_gmic().expect("gmic must be installed to generate previews");
    let source_hash = sha256_file(&cfg.source).expect("source image must exist");
    let version = gmic_version(&gmic);

    let cat = catalogue::builtin();
    let mut jobs = Vec::new();
    collect_jobs(&cat.root, &mut jobs);
    if let Some(only) = &cfg.only {
        jobs.retain(|j| &j.command == only);
    }

    let prev = manifest::load(&cfg.manifest);
    // If the global inputs changed, every entry's hash will differ, so
    // we don't special-case it — but we DO carry the old entries so
    // unchanged filters short-circuit.
    let old_entries = prev.entries;

    let new_entries: Mutex<BTreeMap<String, Entry>> = Mutex::new(BTreeMap::new());
    let recomputed = AtomicUsize::new(0);
    let kept = AtomicUsize::new(0);
    let skipped = AtomicUsize::new(0);

    // Simple fixed-size worker pool over a shared job index.
    let next = AtomicUsize::new(0);
    std::thread::scope(|scope| {
        for _ in 0..cfg.jobs.max(1) {
            scope.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                let Some(job) = jobs.get(i) else { break };
                let hash = manifest::input_hash(&source_hash, &version, &job.command, &job.args);
                let png_path = cfg.out.join(format!("{}.png", job.key));
                let existing = old_entries.get(&job.command);
                let action = if cfg.force {
                    Action::Recompute
                } else {
                    manifest::decide(existing, &hash, png_path.exists())
                };
                let entry = match action {
                    Action::Keep => {
                        kept.fetch_add(1, Ordering::Relaxed);
                        existing.cloned().expect("Keep implies an existing entry")
                    }
                    Action::Recompute => render_one(
                        &gmic,
                        &cfg.source,
                        &png_path,
                        job,
                        &hash,
                        &skipped,
                        &recomputed,
                    ),
                };
                new_entries
                    .lock()
                    .unwrap()
                    .insert(job.command.clone(), entry);
            });
        }
    });

    let manifest_out = Manifest {
        source_hash,
        gmic_version: version,
        entries: new_entries.into_inner().unwrap(),
    };
    manifest::save(&cfg.manifest, &manifest_out).expect("write manifest");

    println!(
        "previews: {} recomputed, {} unchanged, {} skipped ({} total)",
        recomputed.load(Ordering::Relaxed),
        kept.load(Ordering::Relaxed),
        skipped.load(Ordering::Relaxed),
        jobs.len(),
    );
}

/// Render a single filter. On any gmic failure the preview is skipped
/// and recorded — the build never aborts on one bad filter.
fn render_one(
    gmic: &Path,
    source: &Path,
    png_path: &Path,
    job: &Job,
    hash: &str,
    skipped: &AtomicUsize,
    recomputed: &AtomicUsize,
) -> Entry {
    let dir = match tempfile::Builder::new().prefix("gmic-preview").tempdir() {
        Ok(d) => d,
        Err(e) => return skip(skipped, hash, format!("tempdir: {e}")),
    };
    // gmic infers RGB(3) output; the sample image is a colour photo.
    // `filter_tokens` reproduces EXACTLY what the picker sends: command
    // prefixed with `-`, args comma-joined into a single quoted token.
    let tokens = gmic::filter_tokens(&job.command, &job.args);
    match gmic::render_with_tokens(gmic, source, png_path, &tokens, 3, dir.path()) {
        Ok(()) if png_path.exists() => {
            recomputed.fetch_add(1, Ordering::Relaxed);
            Entry {
                input_hash: hash.to_string(),
                status: EntryStatus::Ok,
                file: Some(png_path.file_name().unwrap().to_string_lossy().into_owned()),
                reason: None,
            }
        }
        Ok(()) => skip(skipped, hash, "gmic produced no output file".to_string()),
        Err(e) => {
            // A failed render may have left a partial file; remove it.
            let _ = std::fs::remove_file(png_path);
            skip(skipped, hash, describe(&e))
        }
    }
}

fn skip(skipped: &AtomicUsize, hash: &str, reason: String) -> Entry {
    skipped.fetch_add(1, Ordering::Relaxed);
    Entry {
        input_hash: hash.to_string(),
        status: EntryStatus::Skip,
        file: None,
        reason: Some(reason),
    }
}

fn describe(e: &GmicError) -> String {
    match e {
        GmicError::TimedOut { seconds } => format!("timeout after {seconds}s"),
        GmicError::Failed { status } => format!("gmic exit {status:?}"),
        other => format!("{other}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collect_jobs_covers_every_filter() {
        let cat = catalogue::builtin();
        let mut jobs = Vec::new();
        collect_jobs(&cat.root, &mut jobs);
        // Sanity: the bundled catalogue has well over a thousand filters.
        assert!(jobs.len() > 1000, "expected >1000 jobs, got {}", jobs.len());
        // Keys must be unique so no two filters clobber each other's PNG.
        let mut keys: Vec<&str> = jobs.iter().map(|j| j.key.as_str()).collect();
        keys.sort_unstable();
        let before = keys.len();
        keys.dedup();
        assert_eq!(before, keys.len(), "preview keys collided");
    }
}
