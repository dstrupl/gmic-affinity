//! Parsed G'MIC filter catalogue + supporting types.
//!
//! The IR mirrors what gmic-qt's filter browser shows: a tree of
//! folders containing filters, each filter carrying a `command`, an
//! optional `description`, and a flat parameter list.

pub mod exclusion;
pub mod parser;
pub mod reconcile;

use std::sync::OnceLock;

#[derive(Debug, Clone, PartialEq)]
pub struct Catalogue {
    pub root: Folder,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Folder {
    pub name: String,
    pub children: Vec<Node>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    Folder(Folder),
    Filter(Filter),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Filter {
    pub display_name: String,
    pub command: String,
    pub description: Option<String>,
    pub params: Vec<Param>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    pub label: String,
    pub kind: ParamKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ParamKind {
    Int {
        default: i64,
        min: i64,
        max: i64,
    },
    Float {
        default: f64,
        min: f64,
        max: f64,
    },
    Bool {
        default: bool,
    },
    Choice {
        choices: Vec<String>,
        default: usize,
    },
    Color {
        default_rgba: [u8; 4],
    },
    Text {
        default: String,
    },
    Note(String),
    Separator,
    Link {
        label: String,
        url: String,
    },
    /// Internal-only parameter (G'MIC `value(...)`, `button(...)`,
    /// most `_<type>(...)` forms where the host has no useful UI but
    /// still needs to forward a default to the filter on the
    /// command line). Renders as a small read-only label so users
    /// can see what is going to be sent and contributes
    /// [`InternalValue::default`] verbatim when collecting argv.
    Internal {
        label: String,
        default: String,
    },
    Unknown(String),
}

/// What the picker hands back when the user clicks OK.
#[derive(Debug, Clone, PartialEq)]
pub struct ChosenFilter {
    pub command: String,
    pub args: Vec<String>,
}

/// Lazily-decoded bundled catalogue.
///
/// The bytes are pulled in at compile time from
/// `assets/gmic-catalogue.gmic.gz` (tracked via Git LFS). On first
/// call we gunzip + parse once and cache the resulting `Catalogue`
/// for the life of the process.
static BUILTIN: OnceLock<Catalogue> = OnceLock::new();

pub fn builtin() -> &'static Catalogue {
    BUILTIN.get_or_init(|| {
        use std::io::Read;
        const GZ: &[u8] = include_bytes!("../../assets/gmic-catalogue.gmic.gz");
        let mut text = String::new();
        flate2::read::GzDecoder::new(GZ)
            .read_to_string(&mut text)
            .expect("bundled gmic-catalogue.gmic.gz must decompress");
        parser::parse(&text).expect("bundled gmic-catalogue.gmic.gz must parse")
    })
}

/// Return a "Folder / Sub / Filter" display path for the filter whose
/// `command` matches, or `None` if no filter has that command. Used
/// by `PluginMain` when recording a user pick into `Settings`: the
/// recent list shows the human-readable path, not the gmic command.
pub fn lookup_display_path(cat: &Catalogue, command: &str) -> Option<String> {
    fn walk(folder: &Folder, command: &str, path: &mut Vec<String>) -> Option<String> {
        for child in &folder.children {
            match child {
                Node::Folder(f) => {
                    path.push(f.name.clone());
                    if let Some(found) = walk(f, command, path) {
                        return Some(found);
                    }
                    path.pop();
                }
                Node::Filter(f) if f.command == command => {
                    return Some(if path.is_empty() {
                        f.display_name.clone()
                    } else {
                        format!("{} / {}", path.join(" / "), f.display_name)
                    });
                }
                Node::Filter(_) => {}
            }
        }
        None
    }
    walk(&cat.root, command, &mut Vec::new())
}

/// Return a reference to the filter whose `command` matches, or `None`
/// if no filter has that command. Used by `continue_selector` to convert
/// param-aligned values to gmic argv.
pub fn lookup_filter<'a>(cat: &'a Catalogue, command: &str) -> Option<&'a Filter> {
    fn walk<'a>(folder: &'a Folder, command: &str) -> Option<&'a Filter> {
        for child in &folder.children {
            match child {
                Node::Folder(f) => {
                    if let Some(found) = walk(f, command) {
                        return Some(found);
                    }
                }
                Node::Filter(f) if f.command == command => {
                    return Some(f);
                }
                Node::Filter(_) => {}
            }
        }
        None
    }
    walk(&cat.root, command)
}

#[cfg(test)]
mod path_tests {
    use super::*;

    fn fixture() -> Catalogue {
        Catalogue {
            root: Folder {
                name: String::new(),
                children: vec![
                    Node::Folder(Folder {
                        name: "Artistic".to_string(),
                        children: vec![Node::Filter(Filter {
                            display_name: "Paint Brush".to_string(),
                            command: "fx_painting".to_string(),
                            description: None,
                            params: vec![],
                        })],
                    }),
                    Node::Folder(Folder {
                        name: "Effects".to_string(),
                        children: vec![Node::Folder(Folder {
                            name: "Blur".to_string(),
                            children: vec![Node::Filter(Filter {
                                display_name: "Bokeh".to_string(),
                                command: "fx_bokeh".to_string(),
                                description: None,
                                params: vec![],
                            })],
                        })],
                    }),
                ],
            },
        }
    }

    #[test]
    fn finds_top_level_filter() {
        let cat = fixture();
        assert_eq!(
            lookup_display_path(&cat, "fx_painting"),
            Some("Artistic / Paint Brush".to_string()),
        );
    }

    #[test]
    fn finds_nested_filter() {
        let cat = fixture();
        assert_eq!(
            lookup_display_path(&cat, "fx_bokeh"),
            Some("Effects / Blur / Bokeh".to_string()),
        );
    }

    #[test]
    fn missing_command_returns_none() {
        let cat = fixture();
        assert_eq!(lookup_display_path(&cat, "fx_does_not_exist"), None);
    }

    #[test]
    fn lookup_filter_finds_top_level() {
        let cat = fixture();
        let filter = lookup_filter(&cat, "fx_painting");
        assert!(filter.is_some());
        assert_eq!(filter.unwrap().command, "fx_painting");
    }

    #[test]
    fn lookup_filter_finds_nested() {
        let cat = fixture();
        let filter = lookup_filter(&cat, "fx_bokeh");
        assert!(filter.is_some());
        assert_eq!(filter.unwrap().command, "fx_bokeh");
    }

    #[test]
    fn lookup_filter_returns_none_for_missing() {
        let cat = fixture();
        assert_eq!(lookup_filter(&cat, "fx_does_not_exist"), None);
    }
}
