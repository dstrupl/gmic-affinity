//! Line-driven parser for G'MIC's `#@gui` annotation format.
//!
//! Format spec we rely on:
//! - `#@gui FolderPath` opens or selects a folder. `/` nests.
//! - `#@gui Display Name : command[, preview_command]` opens a
//!   filter inside the current folder.
//! - `#@gui : <label> = <typed-default>` appends one Param row to
//!   the currently-open filter.
//! - Any other `#@gui` row is treated as noise for v1.
//! - Lines that don't start with `#@gui` are ignored.

use crate::catalogue::{Catalogue, Filter, Folder, Node, Param, ParamKind};

#[derive(Debug)]
pub enum ParseError {
    OrphanParam {
        line: usize,
        raw: String,
    },
    Malformed {
        line: usize,
        reason: String,
        raw: String,
    },
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OrphanParam { line, raw } => {
                write!(
                    f,
                    "line {line}: parameter row without an open filter: {raw}"
                )
            }
            Self::Malformed { line, reason, raw } => {
                write!(f, "line {line}: {reason}: {raw}")
            }
        }
    }
}

impl std::error::Error for ParseError {}

pub fn parse(input: &str) -> Result<Catalogue, ParseError> {
    let mut state = ParseState::new();
    for (idx, line) in input.lines().enumerate() {
        state.consume(idx + 1, line)?;
    }
    let mut cat = state.finish()?;
    // Drop folder subtrees that ended up with no filters at all. This
    // happens for headers like `#@gui <b>パターン</b>` whose only
    // children are `#@gui_ja …` localised rows that we (correctly)
    // skip, and for any internal-only group whose every entry was
    // gmic-qt-only and got filtered out.
    prune_empty(&mut cat.root);
    sort_tree(&mut cat.root);
    Ok(cat)
}

struct ParseState {
    folder_stack: Vec<Folder>,
    current_filter: Option<Filter>,
    skip_until_next_filter: bool,
}

impl ParseState {
    fn new() -> Self {
        Self {
            folder_stack: vec![Folder {
                name: String::new(),
                children: Vec::new(),
            }],
            current_filter: None,
            skip_until_next_filter: false,
        }
    }

    fn consume(&mut self, line_no: usize, raw: &str) -> Result<(), ParseError> {
        let Some(body) = raw.trim_start().strip_prefix("#@gui") else {
            return Ok(());
        };
        // Reject `#@gui_<lang>` localisation variants (e.g. `#@gui_ja`,
        // `#@gui_zh`). The English `#@gui` line must be followed by
        // whitespace, a colon, or end-of-line — never an identifier
        // character — or we'd otherwise mis-parse Japanese param rows
        // as filter headers and pollute the catalogue.
        if matches!(
            body.chars().next(),
            Some(c) if c.is_ascii_alphanumeric() || c == '_'
        ) {
            return Ok(());
        }
        let body = body.trim_start();

        if let Some(rest) = body.strip_prefix(':') {
            self.consume_param_row(line_no, rest.trim_start())?;
        } else if body.contains(':') {
            self.consume_filter_header(line_no, body)?;
        } else if !body.is_empty() {
            self.consume_folder(body);
        }
        Ok(())
    }

    fn consume_folder(&mut self, body: &str) {
        self.flush_filter();
        self.skip_until_next_filter = false;
        // Sanitize BEFORE splitting on '/' so that close-tag forms like
        // "< / i>" don't get mistaken for nested folders.
        let path = sanitize_display(body.trim());
        // G'MIC's stdlib uses leading underscores as a sort/priority
        // hint (`_Foo` = secondary, `__Foo` = tertiary, …). gmic-qt
        // strips them for display and merges all variants — and the
        // bare segments like `_` and `___` carry no name at all and
        // need to be dropped. We mirror that here so the tree the
        // user sees lines up with the rest of the ecosystem.
        let segments: Vec<&str> = path
            .split('/')
            .map(str::trim)
            .map(strip_leading_underscores)
            .filter(|s| !s.is_empty())
            .collect();
        if segments.is_empty() {
            return;
        }
        // Unwind everything back to root. Each pop is merged into its
        // parent rather than blindly appended so that any previously-
        // declared folder of the same name absorbs the new children
        // (G'MIC's stdlib re-declares folders many times — e.g.
        // `_Testing` shows up ~15× as different contributors append
        // their bits — and we want one tree node per name).
        while self.folder_stack.len() > 1 {
            let done = self.folder_stack.pop().unwrap();
            merge_folder_into(self.folder_stack.last_mut().unwrap(), done);
        }
        // Descend into matching siblings instead of re-creating folders.
        // If a folder with this segment name already exists at the
        // current level, pull it out of the parent (we'll re-insert it
        // on the next unwind) and use it as the active folder.
        for seg in segments {
            let parent = self.folder_stack.last_mut().unwrap();
            let existing = parent.children.iter().position(|c| match c {
                Node::Folder(f) => f.name == seg,
                Node::Filter(_) => false,
            });
            let folder = if let Some(idx) = existing {
                match parent.children.remove(idx) {
                    Node::Folder(f) => f,
                    // The position predicate above guarantees Folder.
                    Node::Filter(_) => unreachable!(),
                }
            } else {
                Folder {
                    name: seg.to_string(),
                    children: Vec::new(),
                }
            };
            self.folder_stack.push(folder);
        }
    }

    fn consume_filter_header(&mut self, _line_no: usize, body: &str) -> Result<(), ParseError> {
        self.flush_filter();
        let (name_part, command_part) = body.split_once(':').unwrap();
        let display_name = sanitize_display(name_part.trim());
        let command = command_part
            .split(',')
            .next()
            .unwrap_or("")
            .trim()
            .to_string();
        if is_gmic_qt_only(&command) {
            // Skip this filter entirely; subsequent ": param" rows
            // until the next folder/filter are silently ignored.
            self.skip_until_next_filter = true;
            return Ok(());
        }
        self.skip_until_next_filter = false;
        self.current_filter = Some(Filter {
            display_name,
            command,
            description: None,
            params: Vec::new(),
        });
        Ok(())
    }

    fn consume_param_row(&mut self, line_no: usize, body: &str) -> Result<(), ParseError> {
        if self.skip_until_next_filter {
            return Ok(());
        }
        let Some(filter) = self.current_filter.as_mut() else {
            return Err(ParseError::OrphanParam {
                line: line_no,
                raw: body.to_string(),
            });
        };
        let Some((label_part, decl_part)) = body.split_once('=') else {
            let clean = sanitize_display(body.trim());
            filter.params.push(Param {
                label: clean.clone(),
                kind: ParamKind::Note(clean),
            });
            return Ok(());
        };
        // G'MIC convention: a label of `_` (or `_` with leading
        // underscores) marks an anonymous row. The visible content
        // belongs to the value (a Note body, a separator, a hidden
        // `value(0)` placeholder, ...). Treat that as an empty
        // label so the form pane never renders a stray `_` glyph or
        // takes label-column space for it.
        let label_raw = sanitize_display(label_part.trim());
        let label = if label_raw.trim_matches('_').is_empty() {
            String::new()
        } else {
            label_raw
        };
        let kind = parse_kind_for_param(&label, decl_part.trim());
        filter.params.push(Param { label, kind });
        Ok(())
    }

    fn flush_filter(&mut self) {
        if let Some(filter) = self.current_filter.take() {
            self.folder_stack
                .last_mut()
                .unwrap()
                .children
                .push(Node::Filter(filter));
        }
    }

    fn finish(mut self) -> Result<Catalogue, ParseError> {
        self.flush_filter();
        while self.folder_stack.len() > 1 {
            let done = self.folder_stack.pop().unwrap();
            merge_folder_into(self.folder_stack.last_mut().unwrap(), done);
        }
        let root = self.folder_stack.pop().unwrap();
        Ok(Catalogue { root })
    }
}

/// Insert `child` into `parent.children`, merging into any existing
/// folder of the same name (recursively) rather than appending a
/// duplicate sibling. Filters are always appended.
fn merge_folder_into(parent: &mut Folder, child: Folder) {
    let existing = parent.children.iter().position(|c| match c {
        Node::Folder(f) => f.name == child.name,
        Node::Filter(_) => false,
    });
    match existing {
        Some(idx) => {
            let target = match &mut parent.children[idx] {
                Node::Folder(f) => f,
                // The position predicate above guarantees Folder.
                Node::Filter(_) => unreachable!(),
            };
            for grand in child.children {
                match grand {
                    Node::Folder(f) => merge_folder_into(target, f),
                    Node::Filter(f) => target.children.push(Node::Filter(f)),
                }
            }
        }
        None => parent.children.push(Node::Folder(child)),
    }
}

/// Walk the tree and drop any folder subtree that ended up containing
/// zero filters. Called once at the end of [`parse`].
fn prune_empty(folder: &mut Folder) {
    for child in folder.children.iter_mut() {
        if let Node::Folder(sub) = child {
            prune_empty(sub);
        }
    }
    folder.children.retain(|c| match c {
        Node::Folder(f) => !f.children.is_empty(),
        Node::Filter(_) => true,
    });
}

/// Sort every level of the tree alphabetically (case-insensitive),
/// folders first then filters. The catalogue's source order is "the
/// order #@gui blocks appear in update<ver>.gmic", which is roughly
/// historical / author-grouped and unhelpful for navigation — users
/// expect to scroll a long list of community filters alphabetically.
/// We keep folders before filters at the same level so the
/// disclosure-triangle items always sit above the leaf rows, matching
/// the macOS Finder convention.
fn sort_tree(folder: &mut Folder) {
    folder.children.sort_by(|a, b| match (a, b) {
        (Node::Folder(x), Node::Folder(y)) => natural_cmp(&x.name, &y.name),
        (Node::Filter(x), Node::Filter(y)) => natural_cmp(&x.display_name, &y.display_name),
        // Folders sort before filters at the same level.
        (Node::Folder(_), Node::Filter(_)) => std::cmp::Ordering::Less,
        (Node::Filter(_), Node::Folder(_)) => std::cmp::Ordering::Greater,
    });
    for child in folder.children.iter_mut() {
        if let Node::Folder(sub) = child {
            sort_tree(sub);
        }
    }
}

/// Case-insensitive lexicographic compare with a stable tiebreaker
/// on the original casing so two names that differ only in case keep
/// a deterministic order between catalogue refreshes (otherwise the
/// snapshot diff in `make refresh-catalogue` becomes noisy).
fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    a.to_lowercase()
        .cmp(&b.to_lowercase())
        .then_with(|| a.cmp(b))
}

/// Filters whose primary command matches any of these patterns are
/// excluded from the catalogue — they require gmic-qt's IPC and fail
/// headlessly. Documented in plan §9 risk #4.
fn is_gmic_qt_only(command: &str) -> bool {
    command.starts_with("gmic_qt_") || command.starts_with("_gmic_qt_") || command.starts_with('_')
}

struct KindDecl<'a> {
    head: &'a str,
    inner: &'a str,
}

fn split_kind_decl(decl: &str) -> Option<KindDecl<'_>> {
    // G'MIC accepts either `kind(args)` or `kind{args}` as the
    // grouping syntax. Choose whichever opener appears first so we do
    // not latch onto a `(` inside a brace body such as `note{"(...)"}`.
    let paren = decl.find('(');
    let brace = decl.find('{');
    let (open_idx, open_ch, close_ch) = match (paren, brace) {
        (Some(p), Some(b)) if b < p => (b, '{', '}'),
        (Some(p), _) => (p, '(', ')'),
        (None, Some(b)) => (b, '{', '}'),
        (None, None) => return None,
    };
    let (head, rest) = decl.split_at(open_idx);
    let inner = rest
        .strip_prefix(open_ch)
        .and_then(|s| s.rsplit_once(close_ch))
        .map(|(args, _)| args)
        .unwrap_or("");
    Some(KindDecl { head, inner })
}

fn normalized_kind_head(head: &str) -> &str {
    // `~kind(...)` marks gmic-qt "advanced"; `_kind(...)` marks silent.
    // We surface both as ordinary controls for now.
    head.trim().trim_start_matches(['~', '_'])
}

fn parse_kind_for_param(label: &str, decl: &str) -> ParamKind {
    if is_preview_progression_param(label, decl) {
        return ParamKind::Internal {
            label: "headless-preview".to_string(),
            default: "0".to_string(),
        };
    }
    parse_kind(decl)
}

fn is_preview_progression_param(label: &str, decl: &str) -> bool {
    let label = label.to_ascii_lowercase();
    label.contains("preview")
        && label.contains("progress")
        && decl.trim_start().starts_with("_bool")
}

fn parse_kind(decl: &str) -> ParamKind {
    let Some(kind) = split_kind_decl(decl) else {
        return ParamKind::Unknown(sanitize_display(decl));
    };

    match normalized_kind_head(kind.head) {
        "int" => parse_int(kind.inner),
        "float" => parse_float(kind.inner),
        "bool" => parse_bool(kind.inner),
        "choice" => parse_choice(kind.inner),
        "color" => parse_color(kind.inner),
        "text" => parse_text(kind.inner),
        // Note bodies routinely embed HTML markup like
        // `<small><b>Author: ...</b></small>` — sanitise so the form
        // never renders raw tags.
        "note" => ParamKind::Note(sanitize_display(strip_quotes(kind.inner))),
        "separator" => ParamKind::Separator,
        "link" => parse_link(kind.inner),
        // T-after-T10: parse new gmic kinds into existing UI controls
        // so the picker stops rendering them as "(unsupported: ...)"
        // — see `src/bin/audit-unsupported.rs` for the prioritisation
        // matrix.
        "point" => parse_point(kind.inner),
        "value" => parse_internal("value", kind.inner),
        "button" => parse_internal("button", kind.inner),
        "file" | "filein" | "fileout" => parse_path(kind.inner),
        "folder" => parse_path(kind.inner),
        _ => ParamKind::Unknown(sanitize_display(decl)),
    }
}

fn parse_int(s: &str) -> ParamKind {
    let parts: Vec<&str> = s.split(',').map(str::trim).collect();
    match parts.as_slice() {
        [d, lo, hi] => match (d.parse(), lo.parse(), hi.parse()) {
            (Ok(default), Ok(min), Ok(max)) => ParamKind::Int { default, min, max },
            _ => ParamKind::Unknown(sanitize_display(&format!("int({s})"))),
        },
        _ => ParamKind::Unknown(sanitize_display(&format!("int({s})"))),
    }
}

fn parse_float(s: &str) -> ParamKind {
    let parts: Vec<&str> = s.split(',').map(str::trim).collect();
    match parts.as_slice() {
        [d, lo, hi] => match (d.parse(), lo.parse(), hi.parse()) {
            (Ok(default), Ok(min), Ok(max)) => ParamKind::Float { default, min, max },
            _ => ParamKind::Unknown(sanitize_display(&format!("float({s})"))),
        },
        _ => ParamKind::Unknown(sanitize_display(&format!("float({s})"))),
    }
}

fn parse_bool(s: &str) -> ParamKind {
    // Some community filters use Python-style `bool(True)` /
    // `bool(False)`; G'MIC itself accepts `1`/`0`/`true`/`false`.
    // Compare case-insensitively so the audit bucket for these
    // (~56 occurrences in v3.7.6) collapses to zero.
    let lower = s.trim().to_ascii_lowercase();
    // Tolerate `bool(default, min, max)` — community filters use that
    // shape (effectively `int(0, 0, 1)`) when they really want a
    // checkbox; we only care about the first arg.
    let first = lower.split(',').next().unwrap_or("").trim();
    match first {
        "true" | "1" => ParamKind::Bool { default: true },
        // G'MIC stdlib emits a fair number of bare `bool()` decls; the
        // documented behaviour is "default to off", which matches what
        // gmic-qt does too. Without this branch ~166 parameters in
        // the bundled snapshot would render as unsupported.
        "" | "false" | "0" => ParamKind::Bool { default: false },
        _ => ParamKind::Unknown(sanitize_display(&format!("bool({s})"))),
    }
}

fn parse_choice(s: &str) -> ParamKind {
    // G'MIC accepts two syntaxes:
    //   choice(default_index, "a", "b", "c")
    //   choice("a", "b", "c")               // implicit default = 0
    // Detect which one we have by peeking at the first item: if it
    // parses as an integer it's the default index; otherwise the
    // first item is a choice label and the default is 0. Without
    // this, a filter like Frame [Cube] (whose orientation uses the
    // implicit-default form) loses its first choice and reports
    // "Mirror-X" as the apparent default.
    let raw: Vec<&str> = split_top_level(s).collect();
    let (default, label_slice) = match raw.split_first() {
        Some((first, rest)) => match first.trim().parse::<usize>() {
            Ok(idx) => (idx, rest),
            // First token isn't a bare integer — it's a quoted
            // label, so treat the whole list as labels with
            // default 0.
            Err(_) => (0, raw.as_slice()),
        },
        None => return ParamKind::Unknown(sanitize_display(&format!("choice({s})"))),
    };
    // Sanitise every choice label — they appear directly in the
    // NSPopUpButton menu and routinely include `<i>...</i>` markup.
    let choices: Vec<String> = label_slice
        .iter()
        .map(|c| sanitize_display(strip_quotes(c)))
        .collect();
    if choices.is_empty() {
        ParamKind::Unknown(sanitize_display(&format!("choice({s})")))
    } else {
        ParamKind::Choice { choices, default }
    }
}

fn parse_color(s: &str) -> ParamKind {
    let trimmed = s.trim();
    // Hex form (`#RGB`, `#RGBA`, `#RRGGBB`, `#RRGGBBAA`) is by far
    // the most common in user-contributed filters — `assets/gmic-
    // catalogue.toc.txt` shows ~950 occurrences in v3.7.6. COMMIT 2:
    // preserve alpha (default to 255 if not present) so gmic receives
    // the full 4-channel RGBA that gmic-qt sends.
    if let Some(hex) = trimmed.strip_prefix('#') {
        if let Some(rgba) = parse_hex_rgba(hex) {
            return ParamKind::Color { default_rgba: rgba };
        }
        return ParamKind::Unknown(sanitize_display(&format!("color({s})")));
    }
    // Comma-separated byte form (`color(255, 128, 0)` or `color(255, 128, 0, 200)`).
    let parts: Vec<&str> = trimmed.split(',').map(str::trim).collect();
    if parts.len() != 3 && parts.len() != 4 {
        return ParamKind::Unknown(sanitize_display(&format!("color({s})")));
    }
    match (parts[0].parse(), parts[1].parse(), parts[2].parse()) {
        (Ok(r), Ok(g), Ok(b)) => {
            let a = if parts.len() == 4 {
                parts[3].parse().unwrap_or(255)
            } else {
                255
            };
            ParamKind::Color {
                default_rgba: [r, g, b, a],
            }
        }
        _ => ParamKind::Unknown(sanitize_display(&format!("color({s})"))),
    }
}

/// Decode `RGB`, `RGBA`, `RRGGBB`, or `RRGGBBAA` hex (case-insensitive,
/// no leading `#`) into an RGBA quad. Alpha defaults to 255 if not present.
fn parse_hex_rgba(hex: &str) -> Option<[u8; 4]> {
    let hex = hex.trim();
    let (r, g, b, a) = match hex.len() {
        3 => {
            // 4-bit-per-channel shorthand: expand each digit (`a` -> `0xaa`).
            let mut chars = hex.chars();
            let r = expand_nibble(chars.next()?)?;
            let g = expand_nibble(chars.next()?)?;
            let b = expand_nibble(chars.next()?)?;
            (r, g, b, 255)
        }
        4 => {
            let mut chars = hex.chars();
            let r = expand_nibble(chars.next()?)?;
            let g = expand_nibble(chars.next()?)?;
            let b = expand_nibble(chars.next()?)?;
            let a = expand_nibble(chars.next()?)?;
            (r, g, b, a)
        }
        6 => {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            (r, g, b, 255)
        }
        8 => {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            let a = u8::from_str_radix(&hex[6..8], 16).ok()?;
            (r, g, b, a)
        }
        _ => return None,
    };
    Some([r, g, b, a])
}

fn expand_nibble(c: char) -> Option<u8> {
    let n = c.to_digit(16)? as u8;
    Some((n << 4) | n)
}

/// G'MIC `point(x, y, removed?, burst?, R, G, B, A, radius)`: only the
/// first two coords are user-meaningful and 0..=100 in percent units.
/// We render them as a single editable text field of the form `x,y`
/// because adding a true two-spinner row would require a new
/// `ParamKind` variant + matching `FormCell` plumbing. Round-tripping
/// happens because gmic only reads as many args as the filter
/// declares, and the saved-args reconcile path treats this as a
/// single-string parameter.
fn parse_point(s: &str) -> ParamKind {
    let parts: Vec<&str> = split_top_level(s).map(str::trim).collect();
    if parts.len() < 2 {
        return ParamKind::Unknown(sanitize_display(&format!("point({s})")));
    }
    ParamKind::Text {
        default: format!("{},{}", parts[0], parts[1]),
    }
}

/// File- or folder-path picker. Today we just expose the default as
/// editable text; a future iteration can add an NSOpenPanel-backed
/// "Browse..." button.
fn parse_path(s: &str) -> ParamKind {
    ParamKind::Text {
        default: sanitize_display(strip_quotes(s)),
    }
}

/// G'MIC internal-state parameters — `value(default)`, `button(size)`
/// — that gmic-qt hides but the filter still reads from the command
/// line. We surface them as a tiny static label so the user can see
/// what's being forwarded, and the form's `collect_values` path
/// emits the same default back into argv.
fn parse_internal(kind: &str, s: &str) -> ParamKind {
    let trimmed = strip_quotes(s.trim());
    ParamKind::Internal {
        label: kind.to_string(),
        default: sanitize_display(trimmed),
    }
}

fn parse_text(s: &str) -> ParamKind {
    // Default text is shown in the editable NSTextField; HTML markup
    // here would be very visible to the user.
    ParamKind::Text {
        default: sanitize_display(strip_quotes(s)),
    }
}

fn parse_link(s: &str) -> ParamKind {
    let mut iter = split_top_level(s);
    // Link label is user-visible (shown in the row); URL is shown
    // verbatim today, so sanitise it too just in case.
    let label = sanitize_display(iter.next().map(strip_quotes).unwrap_or(""));
    let url = sanitize_display(iter.next().map(strip_quotes).unwrap_or(""));
    if label.is_empty() && url.is_empty() {
        ParamKind::Unknown(sanitize_display(&format!("link({s})")))
    } else {
        ParamKind::Link { label, url }
    }
}

fn strip_quotes(s: &str) -> &str {
    s.trim().trim_matches('"')
}

/// Clean up the display text that appears in folder/filter/param labels.
///
/// G'MIC's `#@gui` annotations sometimes embed HTML-ish markup intended
/// for gmic-qt's rich-text label renderer — `<b>...</b>` and
/// `<i>...</i>` are the only forms observed in v3.7.6, frequently
/// written with whitespace around the slash (`< / i>`). Numeric
/// entities like `&#233;` also appear. None of that should leak into
/// the picker as raw text, AND `< / i>`-style close tags would even
/// confuse `consume_folder`'s `/`-split (turning a single localised
/// folder name into a phantom two-deep hierarchy).
///
/// This function:
/// 1. Strips any balanced `<…>` sequence (matching gmic-qt's loose
///    tag forms, including ones with internal whitespace).
/// 2. Decodes a handful of HTML entities (`&amp;`, `&lt;`, `&gt;`,
///    `&quot;`, `&apos;`, `&nbsp;`) plus numeric `&#NNN;` /
///    `&#xHH;` references.
/// 3. Collapses any runs of whitespace introduced by tag removal so
///    the resulting label has a single space between words.
///
/// Unknown entities are emitted verbatim (still escaped) so we never
/// silently corrupt a label. Tags missing a closing `>` are emitted
/// verbatim — they were probably typos and the user is better served
/// seeing something than seeing nothing.
/// Drop any leading `_` characters from a folder segment.
///
/// G'MIC's stdlib uses `_FolderName` to mark a "secondary" version of
/// a folder, `__FolderName` for tertiary, and so on, and gmic-qt's UI
/// strips those prefixes for display while still merging all variants
/// under one node. We do the same so that e.g. `Rendering`,
/// `_Rendering`, `__Rendering`, and `___Rendering` collapse into one
/// "Rendering" folder rather than four sibling rows.
fn strip_leading_underscores(s: &str) -> &str {
    s.trim_start_matches('_')
}

fn sanitize_display(s: &str) -> String {
    // Byte-indexed scan. `<`, `>`, `&`, `;` and `#` are all single-byte
    // ASCII so finding them in a UTF-8 string by byte equality is
    // always safe (UTF-8 continuation bytes are 0x80..=0xBF, and lead
    // bytes are >= 0xC0 — neither range overlaps any of the ASCII
    // sigils we look for). We only ever pull characters across UTF-8
    // boundaries via `chars().next()` / `len_utf8`.
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'<' {
            // Scan up to 64 bytes for a matching '>'. The 64-byte
            // bound keeps an unclosed '<' (probably a typo, not a tag)
            // from swallowing the rest of a long string.
            let scan_end = (i + 1 + 64).min(bytes.len());
            if let Some(rel) = bytes[i + 1..scan_end].iter().position(|&c| c == b'>') {
                i += 1 + rel + 1;
                continue;
            }
            out.push('<');
            i += 1;
            continue;
        }
        if b == b'&' {
            // Entity? Look for ';' within 12 bytes, but only treat the
            // intervening bytes as an entity body if they all look
            // entity-shaped (alphanumeric or '#'). Anything else and
            // the '&' is just a literal ampersand we should pass
            // through — otherwise we'd over-eat content like
            // "A & B" or "& < / i>".
            let scan_end = (i + 1 + 12).min(bytes.len());
            if let Some(rel) = bytes[i + 1..scan_end].iter().position(|&c| c == b';') {
                let name_bytes = &bytes[i + 1..i + 1 + rel];
                let looks_entity = !name_bytes.is_empty()
                    && name_bytes
                        .iter()
                        .all(|&c| c.is_ascii_alphanumeric() || c == b'#');
                if looks_entity {
                    // ASCII-only by construction — from_utf8 cannot fail.
                    let name = std::str::from_utf8(name_bytes).unwrap();
                    if let Some(decoded) = decode_entity(name) {
                        out.push_str(&decoded);
                    } else {
                        // Unknown but entity-shaped: preserve so we
                        // never silently drop content.
                        out.push('&');
                        out.push_str(name);
                        out.push(';');
                    }
                    i += 1 + rel + 1;
                    continue;
                }
            }
            out.push('&');
            i += 1;
            continue;
        }
        // Copy one full UTF-8 character.
        let ch = s[i..]
            .chars()
            .next()
            .expect("byte index in bounds implies at least one char remains");
        out.push(ch);
        i += ch.len_utf8();
    }
    // Convert literal `\n` (the two-character escape G'MIC uses to
    // embed soft line breaks inside a single-line annotation) into
    // an actual newline. Done before the whitespace-collapse pass so
    // we preserve the line break intent while still squashing the
    // surrounding spaces introduced by tag removal. We split / rejoin
    // on real newlines to preserve them through the per-line
    // whitespace collapse below.
    let with_newlines = out.replace("\\n", "\n");
    with_newlines
        .lines()
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .collect::<Vec<_>>()
        .join("\n")
}

fn decode_entity(name: &str) -> Option<String> {
    match name {
        "amp" => Some("&".into()),
        "lt" => Some("<".into()),
        "gt" => Some(">".into()),
        "quot" => Some("\"".into()),
        "apos" => Some("'".into()),
        "nbsp" => Some(" ".into()),
        _ => {
            let rest = name.strip_prefix('#')?;
            let code = if let Some(hex) = rest.strip_prefix(['x', 'X']) {
                u32::from_str_radix(hex, 16).ok()?
            } else {
                rest.parse::<u32>().ok()?
            };
            Some(char::from_u32(code)?.to_string())
        }
    }
}

/// Quote-aware comma split; no escapes (gmic doesn't use them).
fn split_top_level(s: &str) -> impl Iterator<Item = &str> {
    let mut depth = 0i32;
    let mut in_quote = false;
    let mut start = 0;
    let mut out: Vec<&str> = Vec::new();
    let bytes = s.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'"' => in_quote = !in_quote,
            b'(' if !in_quote => depth += 1,
            b')' if !in_quote => depth -= 1,
            b',' if !in_quote && depth == 0 => {
                out.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&s[start..]);
    out.into_iter()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalogue::{Catalogue, Filter, Folder, Node, ParamKind};

    fn first_filter(cat: &Catalogue) -> &Filter {
        fn walk(folder: &Folder) -> Option<&Filter> {
            for child in &folder.children {
                match child {
                    Node::Filter(f) => return Some(f),
                    Node::Folder(f) => {
                        if let Some(found) = walk(f) {
                            return Some(found);
                        }
                    }
                }
            }
            None
        }
        walk(&cat.root).expect("expected at least one filter")
    }

    #[test]
    fn parses_int_param() {
        let cat =
            parse("#@gui Artistic\n#@gui Paint : fx_paint_brush\n#@gui : Radius = int(5,1,30)\n")
                .unwrap();
        let p = &first_filter(&cat).params[0];
        assert_eq!(p.label, "Radius");
        assert_eq!(
            p.kind,
            ParamKind::Int {
                default: 5,
                min: 1,
                max: 30
            }
        );
    }

    #[test]
    fn parses_float_param() {
        let cat = parse(
            "#@gui Artistic\n#@gui Paint : fx_paint_brush\n#@gui : Density (%) = float(50,0,100)\n",
        )
        .unwrap();
        let p = &first_filter(&cat).params[0];
        assert_eq!(p.label, "Density (%)");
        assert_eq!(
            p.kind,
            ParamKind::Float {
                default: 50.0,
                min: 0.0,
                max: 100.0
            }
        );
    }

    #[test]
    fn parses_bool_param() {
        let cat = parse("#@gui A\n#@gui F : f\n#@gui : On = bool(true)\n").unwrap();
        assert_eq!(
            first_filter(&cat).params[0].kind,
            ParamKind::Bool { default: true }
        );
        let cat = parse("#@gui A\n#@gui F : f\n#@gui : On = bool(0)\n").unwrap();
        assert_eq!(
            first_filter(&cat).params[0].kind,
            ParamKind::Bool { default: false }
        );
    }

    #[test]
    fn parses_choice_with_commas_inside_strings() {
        let cat =
            parse("#@gui A\n#@gui F : f\n#@gui : Mode = choice(2,\"Red, Green\",\"Other\")\n")
                .unwrap();
        assert_eq!(
            first_filter(&cat).params[0].kind,
            ParamKind::Choice {
                choices: vec!["Red, Green".into(), "Other".into()],
                default: 2,
            },
        );
    }

    #[test]
    fn parses_color_param() {
        let cat = parse("#@gui A\n#@gui F : f\n#@gui : Tint = color(255,0,128)\n").unwrap();
        assert_eq!(
            first_filter(&cat).params[0].kind,
            ParamKind::Color {
                default_rgba: [255, 0, 128, 255]
            },
        );
    }

    #[test]
    fn parses_color_hex_with_alpha() {
        // `#RRGGBBAA` is by far the most common color form in
        // community filters (~950 occurrences in the bundled
        // snapshot); now preserve alpha instead of dropping it.
        let cat = parse("#@gui A\n#@gui F : f\n#@gui : Border = color(#000000ff)\n").unwrap();
        assert_eq!(
            first_filter(&cat).params[0].kind,
            ParamKind::Color {
                default_rgba: [0, 0, 0, 255]
            },
        );
        let cat = parse("#@gui A\n#@gui F : f\n#@gui : Tint = color(#abc)\n").unwrap();
        assert_eq!(
            first_filter(&cat).params[0].kind,
            ParamKind::Color {
                default_rgba: [0xaa, 0xbb, 0xcc, 255]
            },
        );
    }

    #[test]
    fn parses_color_hex_preserves_alpha() {
        // COMMIT 2: preserve alpha from hex colors
        let cat = parse("#@gui A\n#@gui F : f\n#@gui : C = color(#ffff007f)\n").unwrap();
        assert_eq!(
            first_filter(&cat).params[0].kind,
            ParamKind::Color {
                default_rgba: [255, 255, 0, 127]
            },
        );
    }

    #[test]
    fn parses_color_hex_no_alpha_defaults_255() {
        // Hex without alpha → alpha=255
        let cat = parse("#@gui A\n#@gui F : f\n#@gui : C = color(#000000)\n").unwrap();
        assert_eq!(
            first_filter(&cat).params[0].kind,
            ParamKind::Color {
                default_rgba: [0, 0, 0, 255]
            },
        );
    }

    #[test]
    fn parses_color_comma_rgba() {
        // Comma form with 4 parts preserves alpha
        let cat = parse("#@gui A\n#@gui F : f\n#@gui : C = color(10,20,30,40)\n").unwrap();
        assert_eq!(
            first_filter(&cat).params[0].kind,
            ParamKind::Color {
                default_rgba: [10, 20, 30, 40]
            },
        );
    }

    #[test]
    fn parses_color_comma_rgb_defaults_alpha_255() {
        // Comma form with 3 parts → alpha=255
        let cat = parse("#@gui A\n#@gui F : f\n#@gui : C = color(10,20,30)\n").unwrap();
        assert_eq!(
            first_filter(&cat).params[0].kind,
            ParamKind::Color {
                default_rgba: [10, 20, 30, 255]
            },
        );
    }

    #[test]
    fn parses_underscore_prefixed_kinds() {
        // `_kind(...)` is the "silent" variant — must round-trip into
        // the same ParamKind as the bare form so the form pane stops
        // showing "(unsupported: _bool())" etc.
        let cat = parse(
            "#@gui A\n#@gui F : f\n\
             #@gui : Inner = _int(2,0,9)\n\
             #@gui : Hidden = _bool(true)\n",
        )
        .unwrap();
        let params = &first_filter(&cat).params;
        assert_eq!(
            params[0].kind,
            ParamKind::Int {
                default: 2,
                min: 0,
                max: 9
            }
        );
        assert_eq!(params[1].kind, ParamKind::Bool { default: true });
    }

    #[test]
    fn preview_progression_bool_is_disabled_for_headless_runs() {
        let cat = parse(
            "#@gui Artistic\n#@gui Linify : fx_linify, fx_linify_preview(0)\n\
             #@gui : Density = float(40,0,100)\n\
             #@gui : Spreading = float(2,0,10)\n\
             #@gui : Resolution (%) = float(40,0,100)\n\
             #@gui : Line Opacity = float(10,0,30)\n\
             #@gui : Line Precision = int(24,1,128)\n\
             #@gui : Color Mode = choice(0,\"Subtractive\",\"Additive\")\n\
             #@gui : Preview Progression While Running = _bool(1)\n",
        )
        .unwrap();

        let params = &first_filter(&cat).params;
        assert_eq!(
            params[6].kind,
            ParamKind::Internal {
                label: "headless-preview".into(),
                default: "0".into(),
            }
        );
    }

    #[test]
    fn parses_brace_grouping() {
        // Filters whose note/text bodies contain parens use `{...}`
        // grouping; we need to pick the brace opener even when an
        // inner `(` appears earlier in the line.
        let cat = parse("#@gui A\n#@gui F : f\n#@gui : help = note{\"(Set to 0 to disable)\"}\n")
            .unwrap();
        match &first_filter(&cat).params[0].kind {
            ParamKind::Note(s) => assert!(s.contains("Set to 0"), "got {s:?}"),
            other => panic!("expected Note, got {other:?}"),
        }
    }

    #[test]
    fn parses_point_first_two_coords() {
        let cat = parse("#@gui A\n#@gui F : f\n#@gui : Corner = point(5,10,0,1,255,0,0,128,3)\n")
            .unwrap();
        assert_eq!(
            first_filter(&cat).params[0].kind,
            ParamKind::Text {
                default: "5,10".into()
            }
        );
    }

    #[test]
    fn parses_value_and_button_as_internal() {
        let cat = parse(
            "#@gui A\n#@gui F : f\n\
             #@gui : v = value(42)\n\
             #@gui : b = button(2)\n",
        )
        .unwrap();
        let params = &first_filter(&cat).params;
        assert!(matches!(
            &params[0].kind,
            ParamKind::Internal { default, .. } if default == "42"
        ));
        assert!(matches!(
            &params[1].kind,
            ParamKind::Internal { default, .. } if default == "2"
        ));
    }

    #[test]
    fn parses_loose_bool_forms() {
        // `bool(True)` (Python-style capitalisation) and
        // `bool(0,0,1)` (author overloaded with int args) both appear
        // in v3.7.6 stdlib.
        let cat = parse("#@gui A\n#@gui F : f\n#@gui : x = bool(True)\n").unwrap();
        assert_eq!(
            first_filter(&cat).params[0].kind,
            ParamKind::Bool { default: true }
        );
        let cat = parse("#@gui A\n#@gui F : f\n#@gui : y = bool(0,0,1)\n").unwrap();
        assert_eq!(
            first_filter(&cat).params[0].kind,
            ParamKind::Bool { default: false }
        );
    }

    #[test]
    fn sort_orders_folders_alphabetically_case_insensitively() {
        // Source order intentionally NOT alphabetical so we know the
        // sort actually ran (rather than being a no-op on already-
        // ordered input). The inner filters double as a check that
        // sort recurses into folders.
        let cat = parse(
            "#@gui Zeta\n\
             #@gui zfx : zfx_cmd\n\
             #@gui afx : afx_cmd\n\
             #@gui Alpha\n\
             #@gui yankee : y_cmd\n\
             #@gui Bravo : b_cmd\n",
        )
        .unwrap();
        let top: Vec<&str> = cat
            .root
            .children
            .iter()
            .filter_map(|n| match n {
                Node::Folder(f) => Some(f.name.as_str()),
                Node::Filter(_) => None,
            })
            .collect();
        assert_eq!(top, vec!["Alpha", "Zeta"], "top-level folders sorted");

        // Inside Alpha we expect Bravo, then yankee (B before y,
        // case-insensitive).
        let alpha = cat
            .root
            .children
            .iter()
            .find_map(|n| match n {
                Node::Folder(f) if f.name == "Alpha" => Some(f),
                _ => None,
            })
            .unwrap();
        let alpha_leaves: Vec<&str> = alpha
            .children
            .iter()
            .filter_map(|n| match n {
                Node::Filter(f) => Some(f.display_name.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(alpha_leaves, vec!["Bravo", "yankee"]);

        // Inside Zeta we expect afx before zfx (sort recursed).
        let zeta = cat
            .root
            .children
            .iter()
            .find_map(|n| match n {
                Node::Folder(f) if f.name == "Zeta" => Some(f),
                _ => None,
            })
            .unwrap();
        let zeta_leaves: Vec<&str> = zeta
            .children
            .iter()
            .filter_map(|n| match n {
                Node::Filter(f) => Some(f.display_name.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(zeta_leaves, vec!["afx", "zfx"]);
    }

    #[test]
    fn bundled_catalogue_has_no_unsupported_params() {
        // Lock the audit invariant: every parameter in the shipped
        // snapshot resolves to a real ParamKind. If gmic ships a new
        // syntax we'll see this fail on the next `make refresh-
        // catalogue` and can decide whether to add a parser arm or
        // intentionally widen the test. See `src/bin/audit-
        // unsupported.rs` for the diagnostic that motivated this
        // guard.
        let cat = crate::catalogue::builtin();
        let mut offenders: Vec<String> = Vec::new();
        fn walk(folder: &Folder, path: &[&str], out: &mut Vec<String>) {
            for child in &folder.children {
                match child {
                    Node::Folder(f) => {
                        let mut p = path.to_vec();
                        p.push(&f.name);
                        walk(f, &p, out);
                    }
                    Node::Filter(f) => {
                        for param in &f.params {
                            if let ParamKind::Unknown(raw) = &param.kind {
                                out.push(format!(
                                    "{} / {} :: {} = {raw}",
                                    path.join(" / "),
                                    f.display_name,
                                    param.label
                                ));
                            }
                        }
                    }
                }
            }
        }
        walk(&cat.root, &[], &mut offenders);
        assert!(
            offenders.is_empty(),
            "bundled catalogue still has {} unsupported param(s) — first 5: {:#?}",
            offenders.len(),
            offenders.iter().take(5).collect::<Vec<_>>()
        );
    }

    #[test]
    fn parses_text_with_quotes() {
        let cat = parse("#@gui A\n#@gui F : f\n#@gui : Caption = text(\"Hello\")\n").unwrap();
        assert_eq!(
            first_filter(&cat).params[0].kind,
            ParamKind::Text {
                default: "Hello".into()
            },
        );
    }

    #[test]
    fn parses_note_and_separator_and_link() {
        let cat = parse(
            "#@gui A\n#@gui F : f\n\
             #@gui : note = note(\"author\")\n\
             #@gui : sep = separator()\n\
             #@gui : help = link(\"docs\",\"https://example.com\")\n",
        )
        .unwrap();
        let params = &first_filter(&cat).params;
        assert_eq!(params[0].kind, ParamKind::Note("author".into()));
        assert_eq!(params[1].kind, ParamKind::Separator);
        assert_eq!(
            params[2].kind,
            ParamKind::Link {
                label: "docs".into(),
                url: "https://example.com".into(),
            },
        );
    }

    #[test]
    fn unknown_kind_does_not_fail_the_parse() {
        let cat = parse("#@gui A\n#@gui F : f\n#@gui : X = wat(1,2,3)\n").unwrap();
        assert!(matches!(
            first_filter(&cat).params[0].kind,
            ParamKind::Unknown(_)
        ));
    }

    #[test]
    fn orphan_param_is_an_error() {
        assert!(matches!(
            parse("#@gui : Radius = int(1,0,10)\n"),
            Err(ParseError::OrphanParam { .. }),
        ));
    }

    #[test]
    fn nested_folders_via_slashes() {
        let cat = parse("#@gui Artistic/Painting\n#@gui Oil : fx_oilpaint\n").unwrap();
        let outer = match &cat.root.children[0] {
            Node::Folder(f) => f,
            _ => panic!("expected folder"),
        };
        assert_eq!(outer.name, "Artistic");
        let inner = match &outer.children[0] {
            Node::Folder(f) => f,
            _ => panic!("expected nested folder"),
        };
        assert_eq!(inner.name, "Painting");
        assert!(matches!(inner.children[0], Node::Filter(_)));
    }

    #[test]
    fn two_filters_under_one_folder() {
        let cat = parse(
            "#@gui Artistic\n\
             #@gui Paint : fx_paint_brush\n\
             #@gui Oil : fx_oilpaint\n",
        )
        .unwrap();
        let folder = match &cat.root.children[0] {
            Node::Folder(f) => f,
            _ => panic!(),
        };
        assert_eq!(folder.children.len(), 2);
    }

    #[test]
    fn whitespace_only_lines_are_ignored() {
        let cat = parse("\n\n#@gui A\n\n   \n#@gui F : f\n").unwrap();
        assert_eq!(first_filter(&cat).command, "f");
    }

    #[test]
    fn comment_after_filter_command_strips_preview() {
        let cat = parse("#@gui A\n#@gui F : fx_real, fx_real_preview(0)\n").unwrap();
        assert_eq!(first_filter(&cat).command, "fx_real");
    }

    #[test]
    fn localised_gmic_lang_prefix_is_ignored() {
        // `#@gui_ja` etc are Japanese-localised mirrors of the catalogue;
        // we only want the English `#@gui` entries.
        let cat = parse(
            "#@gui Cat\n\
             #@gui_ja Cat\n\
             #@gui English Filter : fx_eng\n\
             #@gui_ja Japanese Filter : fx_eng_ja\n\
             #@gui_ja :Coherence=float(100,0,200)\n",
        )
        .unwrap();
        let folder = match &cat.root.children[0] {
            Node::Folder(f) => f,
            _ => panic!("expected the English `Cat` folder"),
        };
        let commands: Vec<&str> = folder
            .children
            .iter()
            .filter_map(|c| match c {
                Node::Filter(f) => Some(f.command.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(commands, vec!["fx_eng"]);
    }

    #[test]
    fn sanitize_strips_bold_and_italic_tags() {
        assert_eq!(sanitize_display("<b>Animals</b>"), "Animals");
        assert_eq!(sanitize_display("<i>Arrays & Tiles</i>"), "Arrays & Tiles");
    }

    #[test]
    fn sanitize_strips_close_tags_with_internal_whitespace() {
        // gmic-qt sometimes writes close tags as `< / i>` — that form
        // even broke `consume_folder`'s `/`-split before this lived in
        // the parser.
        assert_eq!(
            sanitize_display("<i>Arrays & Tiles< / i>"),
            "Arrays & Tiles",
        );
        assert_eq!(sanitize_display("<b>Animals< / b>"), "Animals");
    }

    #[test]
    fn sanitize_decodes_numeric_and_named_entities() {
        assert_eq!(sanitize_display("caf&#233;"), "café");
        assert_eq!(sanitize_display("caf&#xE9;"), "café");
        assert_eq!(sanitize_display("A &amp; B"), "A & B");
        assert_eq!(sanitize_display("&lt;tag&gt;"), "<tag>");
        assert_eq!(sanitize_display("&nbsp;x"), "x"); // nbsp collapses with run
    }

    #[test]
    fn sanitize_preserves_unknown_entities_verbatim() {
        // Don't silently drop content we don't recognise; keep the
        // original spelling so the user can spot it.
        assert_eq!(sanitize_display("&bogus;"), "&bogus;");
    }

    #[test]
    fn sanitize_collapses_whitespace_left_by_tag_removal() {
        assert_eq!(sanitize_display("<b>Foo</b>  <i>Bar</i>"), "Foo Bar");
    }

    #[test]
    fn sanitize_converts_literal_backslash_n_to_newline() {
        // G'MIC routinely embeds soft line breaks as the two-character
        // sequence `\n` inside a single-line `#@gui` annotation. We
        // want the form to render an actual line break instead of the
        // literal sequence.
        assert_eq!(
            sanitize_display("first line\\nsecond line"),
            "first line\nsecond line",
        );
        // ...and we still collapse intra-line whitespace introduced
        // by tag removal on both sides of the break.
        assert_eq!(
            sanitize_display("<b>A</b>  <i>B</i>\\n<b>C</b>  D"),
            "A B\nC D",
        );
    }

    #[test]
    fn sanitize_strips_html_inside_note_bodies_via_parse_kind() {
        // The previous bug: parse_kind's "note" arm stored the raw
        // body, so a note like `note("<small><b>Author: x</b></small>")`
        // surfaced as literal markup in the form pane.
        let kind = parse_kind("note(\"<small><b>Author:</b> Foo</small>\")");
        assert_eq!(kind, ParamKind::Note("Author: Foo".to_string()));
    }

    #[test]
    fn consume_param_row_treats_underscore_label_as_anonymous() {
        // G'MIC uses `_` (or `__`, `___`, …) as the param label for
        // "anonymous" rows whose payload is in the value (notes,
        // separators, hidden placeholders). Without this
        // normalisation the form pane would render a literal `_`
        // label next to every Note and separator.
        let cat = parse(
            "#@gui Cat\n\
             #@gui F : fx\n\
             #@gui :_=note(\"hello\")\n\
             #@gui :__=separator()\n",
        )
        .unwrap();
        let folder = match &cat.root.children[0] {
            Node::Folder(f) => f,
            _ => panic!("expected folder"),
        };
        let filter = match &folder.children[0] {
            Node::Filter(f) => f,
            _ => panic!("expected filter"),
        };
        assert_eq!(filter.params[0].label, "");
        assert_eq!(filter.params[1].label, "");
    }

    #[test]
    fn parse_kind_strips_tilde_advanced_prefix() {
        // G'MIC marks "advanced" params with a leading `~` on the
        // type name. The arg syntax is identical to the un-prefixed
        // form so we parse it normally — anything else means the
        // form pane renders every advanced row as
        // `(unsupported: ~float(...))`, which is what we shipped
        // before this fix.
        assert!(matches!(
            parse_kind("~float(3,0,30)"),
            ParamKind::Float {
                default: 3.0,
                min: 0.0,
                max: 30.0
            }
        ));
        assert!(matches!(
            parse_kind("~int(5,1,10)"),
            ParamKind::Int {
                default: 5,
                min: 1,
                max: 10
            }
        ));
        // G'MIC choice syntax with an explicit default index.
        match parse_kind("~choice(0,\"Normal\",\"Mirror-X\")") {
            ParamKind::Choice {
                ref choices,
                default,
            } => {
                assert_eq!(default, 0);
                assert_eq!(choices, &["Normal", "Mirror-X"]);
            }
            other => panic!("expected Choice, got {other:?}"),
        }
        // ...and the implicit-default form (Frame [Cube] uses this).
        match parse_kind("~choice(\"Normal\",\"Mirror-X\",\"Mirror-Y\")") {
            ParamKind::Choice {
                ref choices,
                default,
            } => {
                assert_eq!(default, 0);
                assert_eq!(choices, &["Normal", "Mirror-X", "Mirror-Y"]);
            }
            other => panic!("expected Choice, got {other:?}"),
        }
        assert!(matches!(
            parse_kind("~bool(true)"),
            ParamKind::Bool { default: true }
        ));
    }

    #[test]
    fn sanitized_close_tag_does_not_create_phantom_folder() {
        // Before the parser-side sanitizer, the `< / i>` form inside a
        // folder path would survive into `path.split('/')` and produce
        // a spurious nested folder. This regression test pins that down.
        let cat = parse("#@gui <i>Lights & Shadows< / i>\n#@gui F : fx\n").unwrap();
        assert_eq!(cat.root.children.len(), 1);
        let folder = match &cat.root.children[0] {
            Node::Folder(f) => f,
            _ => panic!("expected one top-level folder"),
        };
        assert_eq!(folder.name, "Lights & Shadows");
        assert_eq!(folder.children.len(), 1, "no phantom nesting");
    }

    #[test]
    fn filter_display_name_is_sanitised() {
        let cat = parse("#@gui Cat\n#@gui <b>Bold Filter</b> : fx_bold\n").unwrap();
        assert_eq!(first_filter(&cat).display_name, "Bold Filter");
    }

    #[test]
    fn gmic_qt_only_filters_are_excluded() {
        let src = "#@gui Cat\n\
                   #@gui Visible : fx_visible\n\
                   #@gui Hidden  : _gmic_qt_internal\n\
                   #@gui Also Hidden : gmic_qt_dialog\n\
                   #@gui : Density = int(5,0,10)\n\
                   #@gui Visible2 : fx_visible2\n";
        let cat = parse(src).unwrap();
        let folder = match &cat.root.children[0] {
            Node::Folder(f) => f,
            _ => panic!(),
        };
        let commands: Vec<&str> = folder
            .children
            .iter()
            .filter_map(|c| match c {
                Node::Filter(f) => Some(f.command.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(commands, vec!["fx_visible", "fx_visible2"]);
    }

    #[test]
    fn folder_redeclared_at_root_is_merged() {
        // G'MIC's stdlib re-opens the same top-level folder many times
        // as different contributors append their own filters; the
        // catalogue must collapse those into a single node.
        let src = "#@gui Repair\n\
                   #@gui First : fx_first\n\
                   #@gui Other\n\
                   #@gui Mid : fx_mid\n\
                   #@gui Repair\n\
                   #@gui Second : fx_second\n";
        let cat = parse(src).unwrap();
        let names: Vec<&str> = cat
            .root
            .children
            .iter()
            .filter_map(|c| match c {
                Node::Folder(f) => Some(f.name.as_str()),
                _ => None,
            })
            .collect();
        // No duplicate "Repair" entry.
        assert_eq!(
            names.iter().filter(|n| **n == "Repair").count(),
            1,
            "expected one Repair folder, got {:?}",
            names
        );
        let repair = cat
            .root
            .children
            .iter()
            .find_map(|c| match c {
                Node::Folder(f) if f.name == "Repair" => Some(f),
                _ => None,
            })
            .unwrap();
        let cmds: Vec<&str> = repair
            .children
            .iter()
            .filter_map(|c| match c {
                Node::Filter(f) => Some(f.command.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(cmds, vec!["fx_first", "fx_second"]);
    }

    #[test]
    fn folder_redeclared_with_nested_path_is_merged() {
        // Re-opening `A/X` must reuse both `A` and the existing `X`
        // child rather than creating either as a duplicate sibling.
        let src = "#@gui A/X\n\
                   #@gui One : fx_one\n\
                   #@gui A/X\n\
                   #@gui Two : fx_two\n";
        let cat = parse(src).unwrap();
        assert_eq!(cat.root.children.len(), 1, "only one top-level folder");
        let a = match &cat.root.children[0] {
            Node::Folder(f) => f,
            _ => panic!("expected folder A"),
        };
        assert_eq!(a.name, "A");
        assert_eq!(a.children.len(), 1, "only one X child");
        let x = match &a.children[0] {
            Node::Folder(f) => f,
            _ => panic!("expected folder X"),
        };
        assert_eq!(x.name, "X");
        let cmds: Vec<&str> = x
            .children
            .iter()
            .filter_map(|c| match c {
                Node::Filter(f) => Some(f.command.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(cmds, vec!["fx_one", "fx_two"]);
    }

    #[test]
    fn empty_folder_headers_are_pruned() {
        // The Japanese-localised section of G'MIC's stdlib uses
        // `#@gui <b>パターン</b>` headers whose only children are
        // `#@gui_ja …` rows that we skip — so the bare header is
        // left with zero filters and must be dropped from the tree.
        let src = "#@gui Empty Header\n\
                   #@gui_ja Localised : fx_local\n\
                   #@gui Real\n\
                   #@gui Visible : fx_visible\n";
        let cat = parse(src).unwrap();
        let names: Vec<&str> = cat
            .root
            .children
            .iter()
            .filter_map(|c| match c {
                Node::Folder(f) => Some(f.name.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(names, vec!["Real"], "empty header must be pruned");
    }

    #[test]
    fn folders_with_underscore_prefixes_merge_into_clean_name() {
        // G'MIC's stdlib publishes the same logical category under
        // multiple priority levels — `Artistic`, `_Artistic`,
        // `__Artistic` — and the picker must show one folder labelled
        // "Artistic" containing the union of their filters.
        let src = "#@gui Artistic\n\
                   #@gui Real : fx_real\n\
                   #@gui _Artistic\n\
                   #@gui Secondary : fx_secondary\n\
                   #@gui __Artistic\n\
                   #@gui Tertiary : fx_tertiary\n";
        let cat = parse(src).unwrap();
        assert_eq!(cat.root.children.len(), 1, "single merged folder");
        let folder = match &cat.root.children[0] {
            Node::Folder(f) => f,
            _ => panic!("expected folder"),
        };
        assert_eq!(folder.name, "Artistic", "display name has no leading _");
        let cmds: Vec<&str> = folder
            .children
            .iter()
            .filter_map(|c| match c {
                Node::Filter(f) => Some(f.command.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(cmds, vec!["fx_real", "fx_secondary", "fx_tertiary"]);
    }

    #[test]
    fn folder_segments_that_strip_to_empty_are_dropped() {
        // `#@gui _` (or `___`) carries no name once underscores are
        // stripped; emitting a folder named "" would render as a row
        // with no label and an open disclosure triangle.
        let src = "#@gui _\n\
                   #@gui Orphan : fx_orphan\n\
                   #@gui Real\n\
                   #@gui Visible : fx_visible\n";
        let cat = parse(src).unwrap();
        let names: Vec<&str> = cat
            .root
            .children
            .iter()
            .filter_map(|c| match c {
                Node::Folder(f) => Some(f.name.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(
            names,
            vec!["Real"],
            "the `_`-only segment is dropped and its lone filter \
             ends up at the previous scope; tree still has just the \
             one real folder"
        );
    }

    #[test]
    fn nested_folder_is_pruned_when_all_descendants_empty() {
        let src = "#@gui Outer/InnerEmpty\n\
                   #@gui_ja Skipped : fx_skip\n\
                   #@gui Outer/InnerReal\n\
                   #@gui Keep : fx_keep\n";
        let cat = parse(src).unwrap();
        let outer = match &cat.root.children[0] {
            Node::Folder(f) => f,
            _ => panic!(),
        };
        assert_eq!(outer.name, "Outer");
        let names: Vec<&str> = outer
            .children
            .iter()
            .filter_map(|c| match c {
                Node::Folder(f) => Some(f.name.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(names, vec!["InnerReal"], "empty inner pruned");
    }
}
