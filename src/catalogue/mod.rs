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
