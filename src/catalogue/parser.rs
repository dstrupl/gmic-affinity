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
    OrphanParam { line: usize, raw: String },
    Malformed { line: usize, reason: String, raw: String },
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OrphanParam { line, raw } => {
                write!(f, "line {line}: parameter row without an open filter: {raw}")
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
        let label = sanitize_display(label_part.trim());
        let kind = parse_kind(decl_part.trim());
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

/// Filters whose primary command matches any of these patterns are
/// excluded from the catalogue — they require gmic-qt's IPC and fail
/// headlessly. Documented in plan §9 risk #4.
fn is_gmic_qt_only(command: &str) -> bool {
    command.starts_with("gmic_qt_")
        || command.starts_with("_gmic_qt_")
        || command.starts_with('_')
}

fn parse_kind(decl: &str) -> ParamKind {
    let Some(open) = decl.find('(') else {
        return ParamKind::Unknown(decl.to_string());
    };
    let (head, rest) = decl.split_at(open);
    let inner = rest
        .strip_prefix('(')
        .and_then(|s| s.rsplit_once(')'))
        .map(|(args, _)| args)
        .unwrap_or("");
    match head.trim() {
        "int" => parse_int(inner),
        "float" => parse_float(inner),
        "bool" => parse_bool(inner),
        "choice" => parse_choice(inner),
        "color" => parse_color(inner),
        "text" => parse_text(inner),
        "note" => ParamKind::Note(strip_quotes(inner).to_string()),
        "separator" => ParamKind::Separator,
        "link" => parse_link(inner),
        _ => ParamKind::Unknown(decl.to_string()),
    }
}

fn parse_int(s: &str) -> ParamKind {
    let parts: Vec<&str> = s.split(',').map(str::trim).collect();
    match parts.as_slice() {
        [d, lo, hi] => match (d.parse(), lo.parse(), hi.parse()) {
            (Ok(default), Ok(min), Ok(max)) => ParamKind::Int { default, min, max },
            _ => ParamKind::Unknown(format!("int({s})")),
        },
        _ => ParamKind::Unknown(format!("int({s})")),
    }
}

fn parse_float(s: &str) -> ParamKind {
    let parts: Vec<&str> = s.split(',').map(str::trim).collect();
    match parts.as_slice() {
        [d, lo, hi] => match (d.parse(), lo.parse(), hi.parse()) {
            (Ok(default), Ok(min), Ok(max)) => ParamKind::Float { default, min, max },
            _ => ParamKind::Unknown(format!("float({s})")),
        },
        _ => ParamKind::Unknown(format!("float({s})")),
    }
}

fn parse_bool(s: &str) -> ParamKind {
    match s.trim() {
        "true" | "1" => ParamKind::Bool { default: true },
        "false" | "0" => ParamKind::Bool { default: false },
        other => ParamKind::Unknown(format!("bool({other})")),
    }
}

fn parse_choice(s: &str) -> ParamKind {
    let mut iter = split_top_level(s);
    let default = iter
        .next()
        .and_then(|d| d.trim().parse().ok())
        .unwrap_or(0);
    let choices: Vec<String> = iter.map(|c| strip_quotes(c).to_string()).collect();
    if choices.is_empty() {
        ParamKind::Unknown(format!("choice({s})"))
    } else {
        ParamKind::Choice { choices, default }
    }
}

fn parse_color(s: &str) -> ParamKind {
    let parts: Vec<&str> = s.split(',').map(str::trim).collect();
    if parts.len() != 3 {
        return ParamKind::Unknown(format!("color({s})"));
    }
    match (parts[0].parse(), parts[1].parse(), parts[2].parse()) {
        (Ok(r), Ok(g), Ok(b)) => ParamKind::Color {
            default_rgb: [r, g, b],
        },
        _ => ParamKind::Unknown(format!("color({s})")),
    }
}

fn parse_text(s: &str) -> ParamKind {
    ParamKind::Text {
        default: strip_quotes(s).to_string(),
    }
}

fn parse_link(s: &str) -> ParamKind {
    let mut iter = split_top_level(s);
    let label = iter
        .next()
        .map(strip_quotes)
        .unwrap_or("")
        .to_string();
    let url = iter
        .next()
        .map(strip_quotes)
        .unwrap_or("")
        .to_string();
    if label.is_empty() && url.is_empty() {
        ParamKind::Unknown(format!("link({s})"))
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
    // Collapse whitespace runs left over from tag removal so a tag-rich
    // label like "<b>Foo</b>  <i>Bar</i>" comes out as "Foo Bar".
    out.split_whitespace().collect::<Vec<_>>().join(" ")
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
        let cat = parse("#@gui Artistic\n#@gui Paint : fx_paint_brush\n#@gui : Radius = int(5,1,30)\n").unwrap();
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
        let cat = parse("#@gui Artistic\n#@gui Paint : fx_paint_brush\n#@gui : Density (%) = float(50,0,100)\n").unwrap();
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
        let cat = parse(
            "#@gui A\n#@gui F : f\n#@gui : Mode = choice(2,\"Red, Green\",\"Other\")\n",
        )
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
                default_rgb: [255, 0, 128]
            },
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
        assert!(matches!(first_filter(&cat).params[0].kind, ParamKind::Unknown(_)));
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
        let cat = parse(
            "#@gui A\n#@gui F : fx_real, fx_real_preview(0)\n",
        )
        .unwrap();
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
