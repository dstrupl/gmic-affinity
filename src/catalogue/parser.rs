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
    state.finish()
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
        let path = body.trim();
        let segments: Vec<&str> = path.split('/').map(str::trim).collect();
        while self.folder_stack.len() > 1 {
            let done = self.folder_stack.pop().unwrap();
            self.folder_stack
                .last_mut()
                .unwrap()
                .children
                .push(Node::Folder(done));
        }
        for seg in segments {
            self.folder_stack.push(Folder {
                name: seg.to_string(),
                children: Vec::new(),
            });
        }
    }

    fn consume_filter_header(&mut self, _line_no: usize, body: &str) -> Result<(), ParseError> {
        self.flush_filter();
        let (name_part, command_part) = body.split_once(':').unwrap();
        let display_name = name_part.trim().to_string();
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
            filter.params.push(Param {
                label: body.trim().to_string(),
                kind: ParamKind::Note(body.trim().to_string()),
            });
            return Ok(());
        };
        let label = label_part.trim().to_string();
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
            self.folder_stack
                .last_mut()
                .unwrap()
                .children
                .push(Node::Folder(done));
        }
        let root = self.folder_stack.pop().unwrap();
        Ok(Catalogue { root })
    }
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
}
