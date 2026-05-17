//! Dump the parsed bundled catalogue as a stable one-line-per-filter TOC.
//! Used by `make refresh-catalogue` to keep `assets/gmic-catalogue.toc.txt`
//! in sync with what our parser actually understands of the snapshot.

use GmicFilter::catalogue::{self, Folder, Node};

fn main() {
    let cat = catalogue::builtin();
    let mut lines: Vec<String> = Vec::new();
    walk(&cat.root, &mut Vec::new(), &mut lines);
    lines.sort();
    for line in lines {
        println!("{line}");
    }
}

fn walk(folder: &Folder, path: &mut Vec<String>, out: &mut Vec<String>) {
    for child in &folder.children {
        match child {
            Node::Folder(f) => {
                path.push(f.name.clone());
                walk(f, path, out);
                path.pop();
            }
            Node::Filter(f) => {
                let full_path = if path.is_empty() {
                    f.display_name.clone()
                } else {
                    format!("{} / {}", path.join(" / "), f.display_name)
                };
                out.push(format!("{full_path}  ->  {}", f.command));
            }
        }
    }
}
