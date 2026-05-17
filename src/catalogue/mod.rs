//! Parsed G'MIC filter catalogue + supporting types.
//!
//! The IR mirrors what gmic-qt's filter browser shows: a tree of
//! folders containing filters, each filter carrying a `command`, an
//! optional `description`, and a flat parameter list.

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
    Int { default: i64, min: i64, max: i64 },
    Float { default: f64, min: f64, max: f64 },
    Bool { default: bool },
    Choice { choices: Vec<String>, default: usize },
    Color { default_rgb: [u8; 3] },
    Text { default: String },
    Note(String),
    Separator,
    Link { label: String, url: String },
    Unknown(String),
}

/// What the picker hands back when the user clicks OK.
#[derive(Debug, Clone, PartialEq)]
pub struct ChosenFilter {
    pub command: String,
    pub args: Vec<String>,
}

/// Lazily-decoded bundled catalogue. T6 populates `BUNDLED_GZ` with
/// real content; until then we expose a tiny placeholder so this
/// module compiles standalone.
static BUILTIN: OnceLock<Catalogue> = OnceLock::new();

#[cfg(not(test))]
pub fn builtin() -> &'static Catalogue {
    BUILTIN.get_or_init(|| {
        // Populated for real in T6 via include_bytes!() + flate2.
        // Until then, returning an empty catalogue lets the rest of
        // the modules compile and unit-test in isolation.
        Catalogue {
            root: Folder {
                name: String::new(),
                children: Vec::new(),
            },
        }
    })
}

#[cfg(test)]
pub fn builtin() -> &'static Catalogue {
    BUILTIN.get_or_init(|| Catalogue {
        root: Folder {
            name: String::new(),
            children: Vec::new(),
        },
    })
}
